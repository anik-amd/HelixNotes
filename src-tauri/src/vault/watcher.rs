use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};
use notify::{Config, Event, EventKind, PollWatcher, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

use crate::state::AppState;
use crate::types::FileEvent;
use crate::vault::operations::{is_hidden, read_note_entry_fast};

const IOS_POLL_INTERVAL: Duration = Duration::from_secs(10);
const FLATPAK_POLL_INTERVAL: Duration = Duration::from_secs(2);
const NATIVE_POLL_INTERVAL: Duration = Duration::from_secs(1);
const EVENT_BATCH_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WatcherBackend {
    Recommended,
    Poll,
}

const fn watcher_backend_for_target(is_ios: bool, is_flatpak: bool) -> WatcherBackend {
    if is_ios || is_flatpak {
        WatcherBackend::Poll
    } else {
        WatcherBackend::Recommended
    }
}

const fn poll_interval_for_target(is_ios: bool, is_flatpak: bool) -> Duration {
    if is_ios {
        IOS_POLL_INTERVAL
    } else if is_flatpak {
        FLATPAK_POLL_INTERVAL
    } else {
        NATIVE_POLL_INTERVAL
    }
}

pub enum VaultWatcher {
    Recommended(RecommendedWatcher),
    Poll(PollWatcher),
}

impl VaultWatcher {
    fn watch(&mut self, path: &Path, recursive_mode: RecursiveMode) -> notify::Result<()> {
        match self {
            Self::Recommended(watcher) => watcher.watch(path, recursive_mode),
            Self::Poll(watcher) => watcher.watch(path, recursive_mode),
        }
    }
}

fn collect_changes(event: Event, vault_root: &Path, changes: &mut HashMap<PathBuf, &'static str>) {
    let (default_type, may_be_directory) = match event.kind {
        EventKind::Create(kind) => ("create", kind != CreateKind::File),
        EventKind::Remove(kind) => ("remove", kind != RemoveKind::File),
        EventKind::Modify(ModifyKind::Name(_)) => ("modify", true),
        EventKind::Modify(_) => ("modify", false),
        _ => return,
    };
    for (index, path) in event.paths.into_iter().enumerate() {
        let Ok(relative) = path.strip_prefix(vault_root) else {
            continue;
        };
        if !relative.components().all(|component| match component {
            Component::Normal(name) => !is_hidden(Path::new(name)),
            Component::CurDir => true,
            _ => false,
        }) {
            continue;
        }
        // Removed/renamed directories no longer pass is_dir(). Retain unknown
        // missing paths so external notebook changes still reach the UI.
        if path.extension().and_then(|extension| extension.to_str()) != Some("md")
            && !path.is_dir()
            && !(may_be_directory && !path.exists())
        {
            continue;
        }
        let event_type = match event.kind {
            EventKind::Modify(ModifyKind::Name(RenameMode::From)) => "remove",
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if index == 0 => "remove",
            _ => default_type,
        };
        changes.insert(path, event_type);
    }
}

pub(crate) fn changed_entry(path: PathBuf, event_type: &str, vault_root: &Path) -> FileEvent {
    let is_note =
        path.extension().and_then(|extension| extension.to_str()) == Some("md") && path.is_file();
    let note = if is_note {
        read_note_entry_fast(&path, vault_root).ok()
    } else {
        None
    };
    // Atomic replacement may also report removal of the old destination. The
    // final filesystem state, not the platform's rename spelling, is authoritative.
    let event_type = if path.exists() {
        if event_type == "remove" {
            "modify"
        } else {
            event_type
        }
    } else {
        "remove"
    };
    FileEvent {
        vault_path: vault_root.to_string_lossy().to_string(),
        event_type: event_type.to_string(),
        path: path.to_string_lossy().to_string(),
        note,
    }
}

pub fn start_watcher(app: AppHandle, vault_path: String) -> Result<VaultWatcher, String> {
    let (tx, rx) = mpsc::channel();

    let is_ios = cfg!(target_os = "ios");
    // Document-portal FUSE paths expose external changes without forwarding
    // inotify events. Poll only in Flatpak, keeping native file watchers intact.
    let is_flatpak = option_env!("HELIXNOTES_INSTALL_TYPE") == Some("flatpak");
    let backend = watcher_backend_for_target(is_ios, is_flatpak);
    let config = Config::default().with_poll_interval(poll_interval_for_target(is_ios, is_flatpak));
    let mut watcher = match backend {
        WatcherBackend::Recommended => VaultWatcher::Recommended(
            RecommendedWatcher::new(tx, config).map_err(|e| e.to_string())?,
        ),
        WatcherBackend::Poll => {
            VaultWatcher::Poll(PollWatcher::new(tx, config).map_err(|e| e.to_string())?)
        }
    };

    watcher
        .watch(Path::new(&vault_path), RecursiveMode::Recursive)
        .map_err(|e| e.to_string())?;

    std::thread::spawn(move || {
        let vault_root = Path::new(&vault_path);
        let mut changes = HashMap::new();
        let mut search = None;
        loop {
            let first = if changes.is_empty() {
                match rx.recv() {
                    Ok(event) => Some(event),
                    Err(_) => return,
                }
            } else {
                match rx.recv_timeout(EVENT_BATCH_INTERVAL) {
                    Ok(event) => Some(event),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            };
            let deadline = Instant::now() + EVENT_BATCH_INTERVAL;
            let mut next = first;
            while let Some(result) = next {
                match result {
                    Ok(event) => {
                        collect_changes(event, vault_root, &mut changes);
                    }
                    Err(error) => log::error!("File watcher error: {error}"),
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                next = rx.recv_timeout(remaining).ok();
            }
            let state = app.state::<AppState>();
            // Native note mutations hold note_save through their path event.
            // Read and emit only after they finish, so the watcher cannot
            // publish an intermediate rename or stale note contents.
            let _save_guard = state.note_save.blocking_lock();
            // Keep the batch while muted, rather than losing concurrent external
            // edits. Nothing is indexed or emitted until local imports finish.
            if state.importing.load(std::sync::atomic::Ordering::Relaxed) {
                continue;
            }
            // Each watcher belongs to its own vault. A detached note window may
            // still need changes from a vault that is no longer active; never
            // index those changes into the main window's active search index.
            if state.validate_vault(&vault_path).is_err() {
                continue;
            }
            if search.is_none() {
                match state.vault_snapshot(&vault_path) {
                    Ok(index) => search = index,
                    Err(error) => {
                        log::error!("Could not open search index for watched vault: {error}");
                        continue;
                    }
                }
            }
            let mut refresh_vault = false;
            for (path, event_type) in changes.drain() {
                if path.extension().and_then(|extension| extension.to_str()) == Some("md")
                    && !path.is_dir()
                {
                    if let Some(search) = &search {
                        if let Err(error) = search.queue_index_note(&path.to_string_lossy()) {
                            log::error!("Could not queue external note for search: {error}");
                        }
                    }
                } else {
                    refresh_vault = true;
                }
                let event = changed_entry(path, event_type, vault_root);
                let _ = app.emit("file-changed", &event);
            }
            if refresh_vault {
                if let Some(search) = &search {
                    if let Err(error) = search.queue_vault_refresh(&vault_path) {
                        log::error!("Could not queue notebook search refresh: {error}");
                    }
                }
            }
        }
    });

    Ok(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ios_uses_polling_backend() {
        assert_eq!(
            watcher_backend_for_target(true, false),
            WatcherBackend::Poll
        );
    }

    #[test]
    fn other_platforms_keep_recommended_backend() {
        assert_eq!(
            watcher_backend_for_target(false, false),
            WatcherBackend::Recommended
        );
    }

    #[test]
    fn watcher_targets_use_the_expected_intervals() {
        assert_eq!(
            poll_interval_for_target(true, false),
            Duration::from_secs(10)
        );
        assert_eq!(
            poll_interval_for_target(false, true),
            Duration::from_secs(2)
        );
        assert_eq!(
            poll_interval_for_target(false, false),
            Duration::from_secs(1)
        );
    }

    fn mapped_events(events: Vec<Event>, vault_root: &Path) -> Vec<FileEvent> {
        let mut changes = HashMap::new();
        for event in events {
            collect_changes(event, vault_root, &mut changes);
        }
        changes
            .into_iter()
            .map(|(path, event_type)| changed_entry(path, event_type, vault_root))
            .collect()
    }

    #[test]
    fn atomic_replace_emits_only_enriched_destination() {
        let vault = tempfile::tempdir().unwrap();
        let target = vault.path().join("Note.md");
        let temporary = vault.path().join(".helixnotes-save-123");
        std::fs::write(&target, "---\ntitle: New title\n---\nUpdated preview.").unwrap();
        let events = mapped_events(
            vec![
                Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
                    .add_path(temporary)
                    .add_path(target.clone()),
                Event::new(EventKind::Remove(RemoveKind::File)).add_path(target.clone()),
                Event::new(EventKind::Create(CreateKind::File))
                    .add_path(vault.path().join(".helixnotes/history/snapshot.md")),
                Event::new(EventKind::Create(CreateKind::File))
                    .add_path(vault.path().join(".hidden/secret.md")),
            ],
            vault.path(),
        );

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].path, target.to_string_lossy());
        assert_eq!(events[0].event_type, "modify");
        let note = events[0].note.as_ref().unwrap();
        assert_eq!(note.meta.title, "New title");
        assert_eq!(note.relative_path, "Note.md");
        assert_eq!(note.preview, "Updated preview.");
    }

    #[test]
    fn external_note_and_directory_renames_keep_both_sides() {
        let vault = tempfile::tempdir().unwrap();
        let old_note = vault.path().join("Before.md");
        let new_note = vault.path().join("After.md");
        let old_directory = vault.path().join("Old.notebook");
        let new_directory = vault.path().join("New.notebook");
        std::fs::write(&new_note, "renamed note").unwrap();
        std::fs::create_dir(&new_directory).unwrap();
        let events = mapped_events(
            vec![
                Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
                    .add_path(old_note.clone())
                    .add_path(new_note.clone()),
                Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
                    .add_path(old_directory.clone())
                    .add_path(new_directory.clone()),
            ],
            vault.path(),
        );

        for removed in [old_note, old_directory] {
            let event = events
                .iter()
                .find(|event| event.path == removed.to_string_lossy())
                .unwrap();
            assert_eq!(event.event_type, "remove");
            assert!(event.note.is_none());
        }
        let note = events
            .iter()
            .find(|event| event.path == new_note.to_string_lossy())
            .unwrap();
        assert_eq!(note.event_type, "modify");
        assert!(note.note.is_some());
        let directory = events
            .iter()
            .find(|event| event.path == new_directory.to_string_lossy())
            .unwrap();
        assert_eq!(directory.event_type, "modify");
        assert!(directory.note.is_none());
    }

    #[test]
    fn batching_keeps_distinct_external_changes_and_deleted_directories() {
        let vault = tempfile::tempdir().unwrap();
        let first = vault.path().join("First.md");
        let second = vault.path().join("Second.md");
        let removed = vault.path().join("Removed.notebook");
        std::fs::write(&first, "first external change").unwrap();
        std::fs::write(&second, "second external change").unwrap();
        let events = mapped_events(
            vec![
                Event::new(EventKind::Create(CreateKind::File)).add_path(first.clone()),
                Event::new(EventKind::Modify(ModifyKind::Any)).add_path(first.clone()),
                Event::new(EventKind::Create(CreateKind::File)).add_path(second.clone()),
                Event::new(EventKind::Remove(RemoveKind::Folder)).add_path(removed.clone()),
            ],
            vault.path(),
        );

        let first_event = events
            .iter()
            .find(|event| event.path == first.to_string_lossy())
            .unwrap();
        assert_eq!(
            first_event.note.as_ref().unwrap().preview,
            "first external change"
        );
        let second_event = events
            .iter()
            .find(|event| event.path == second.to_string_lossy())
            .unwrap();
        assert_eq!(
            second_event.note.as_ref().unwrap().preview,
            "second external change"
        );
        let removed_event = events
            .iter()
            .find(|event| event.path == removed.to_string_lossy())
            .unwrap();
        assert_eq!(removed_event.event_type, "remove");
        assert!(removed_event.note.is_none());
        assert_eq!(
            events
                .iter()
                .filter(|event| event.path == first.to_string_lossy())
                .count(),
            1
        );
    }
}
