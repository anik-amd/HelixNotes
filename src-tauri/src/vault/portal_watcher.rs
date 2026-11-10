//! Metadata polling for portal directories that do not forward host inotify events.
use notify::event::{CreateKind, ModifyKind, RemoveKind};
use notify::{Event, EventKind};
use std::collections::HashMap;
use std::fs::{self, Metadata};
use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};
use walkdir::WalkDir;

use super::operations::is_hidden;

#[derive(Debug, PartialEq, Eq)]
enum Fingerprint {
    Directory,
    Note {
        size: u64,
        modified: (i64, i64),
        changed: (i64, i64),
    },
}

impl Fingerprint {
    fn from_metadata(metadata: &Metadata) -> Self {
        if metadata.is_dir() {
            // Track directory presence, not child mutations or synthetic portal inode IDs.
            Self::Directory
        } else {
            Self::Note {
                size: metadata.len(),
                modified: (metadata.mtime(), metadata.mtime_nsec()),
                changed: (metadata.ctime(), metadata.ctime_nsec()),
            }
        }
    }
}

type Snapshot = HashMap<PathBuf, Fingerprint>;

fn scan(root: &Path) -> notify::Result<Snapshot> {
    let metadata =
        fs::metadata(root).map_err(|error| notify::Error::io(error).add_path(root.to_owned()))?;
    if !metadata.is_dir() {
        return Err(
            notify::Error::generic("Vault root is not a directory").add_path(root.to_owned())
        );
    }
    let mut snapshot = Snapshot::new();
    let entries = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| entry.depth() == 0 || !is_hidden(entry.path()));
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error)
                if error.depth() > 0
                    && error
                        .io_error()
                        .is_some_and(|error| error.kind() == ErrorKind::NotFound) =>
            {
                continue;
            }
            Err(error) => {
                return Err(notify::Error::generic(&error.to_string())
                    .add_path(error.path().unwrap_or(root).to_owned()));
            }
        };
        let path = entry.path();
        if !entry.file_type().is_dir()
            && path.extension().and_then(|extension| extension.to_str()) != Some("md")
        {
            continue;
        }
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if entry.depth() > 0 && error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(notify::Error::io(error).add_path(path.to_owned())),
        };
        // Match note scanning's file symlinks without traversing directory symlinks.
        if metadata.is_file() || (metadata.is_dir() && !entry.file_type().is_symlink()) {
            snapshot.insert(path.to_owned(), Fingerprint::from_metadata(&metadata));
        }
    }
    Ok(snapshot)
}

fn changes(before: &Snapshot, after: &Snapshot) -> Vec<Event> {
    let mut events = Vec::new();
    for (path, fingerprint) in after {
        let kind = match before.get(path) {
            None => EventKind::Create(CreateKind::Any),
            Some(previous) if previous != fingerprint => EventKind::Modify(ModifyKind::Any),
            Some(_) => continue,
        };
        events.push(Event::new(kind).add_path(path.clone()));
    }
    for path in before.keys().filter(|path| !after.contains_key(*path)) {
        events.push(Event::new(EventKind::Remove(RemoveKind::Any)).add_path(path.clone()));
    }
    events
}

pub struct PortalWatcher {
    _stop: mpsc::Sender<()>,
}

impl PortalWatcher {
    pub fn start(
        root: PathBuf,
        events: mpsc::Sender<notify::Result<Event>>,
        interval: Duration,
    ) -> Result<Self, String> {
        let started = Instant::now();
        let mut previous = scan(&root).map_err(|error| error.to_string())?;
        // Portal metadata calls consume host daemon CPU too. Large vaults trade
        // detection latency for idle time instead of scanning continuously.
        let mut delay = interval.max(started.elapsed().saturating_mul(20));
        let (stop, stopped) = mpsc::channel();
        std::thread::Builder::new()
            .name("portal-vault-watch".into())
            .spawn(move || {
                while matches!(stopped.recv_timeout(delay), Err(RecvTimeoutError::Timeout)) {
                    let started = Instant::now();
                    match scan(&root) {
                        Ok(current) => {
                            for event in changes(&previous, &current) {
                                if events.send(Ok(event)).is_err() {
                                    return;
                                }
                            }
                            previous = current;
                        }
                        Err(error) => {
                            // A partial or inaccessible scan cannot prove that notes were deleted.
                            if events.send(Err(error)).is_err() {
                                return;
                            }
                        }
                    }
                    delay = interval.max(started.elapsed().saturating_mul(20));
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self { _stop: stop })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{File, FileTimes};
    use std::os::unix::fs::symlink;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    fn set_modified(path: &Path, time: SystemTime) {
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(time))
            .unwrap();
    }

    fn assert_modified(before: &Snapshot, root: &Path, note: &Path) {
        let events = changes(before, &scan(root).unwrap());
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].paths, [note]);
        assert_eq!(events[0].kind, EventKind::Modify(ModifyKind::Any));
    }

    #[test]
    fn same_size_edit_with_restored_mtime_uses_ctime() {
        let vault = tempfile::tempdir().unwrap();
        let note = vault.path().join("Note.md");
        fs::write(&note, "before").unwrap();
        let modified = fs::metadata(&note).unwrap().modified().unwrap();
        let before = scan(vault.path()).unwrap();
        std::thread::sleep(Duration::from_millis(2));
        fs::write(&note, "after!").unwrap();
        set_modified(&note, modified);
        assert_eq!(fs::metadata(&note).unwrap().modified().unwrap(), modified);
        assert_modified(&before, vault.path(), &note);
    }

    #[test]
    fn backdated_and_fractional_modification_times_are_detected() {
        let vault = tempfile::tempdir().unwrap();
        let note = vault.path().join("Note.md");
        fs::write(&note, "before").unwrap();
        set_modified(&note, UNIX_EPOCH + Duration::new(1000, 100));
        let before = scan(vault.path()).unwrap();
        fs::write(&note, "after!").unwrap();
        set_modified(&note, UNIX_EPOCH + Duration::new(1000, 200));
        let metadata = fs::metadata(&note).unwrap();
        assert_eq!((metadata.mtime(), metadata.mtime_nsec()), (1000, 200));
        assert_modified(&before, vault.path(), &note);
        let before = scan(vault.path()).unwrap();
        set_modified(&note, UNIX_EPOCH + Duration::from_secs(10));
        assert_modified(&before, vault.path(), &note);
    }

    #[test]
    fn atomic_replacement_with_preserved_mtime_changes_ctime() {
        let vault = tempfile::tempdir().unwrap();
        let note = vault.path().join("Note.md");
        let temporary = vault.path().join(".replacement");
        fs::write(&note, "before").unwrap();
        let metadata = fs::metadata(&note).unwrap();
        let before = scan(vault.path()).unwrap();
        std::thread::sleep(Duration::from_millis(2));
        fs::write(&temporary, "after!").unwrap();
        set_modified(&temporary, metadata.modified().unwrap());
        fs::rename(&temporary, &note).unwrap();
        let replaced = fs::metadata(&note).unwrap();
        assert_eq!(replaced.modified().unwrap(), metadata.modified().unwrap());
        assert_ne!(
            (replaced.ctime(), replaced.ctime_nsec()),
            (metadata.ctime(), metadata.ctime_nsec())
        );
        assert_modified(&before, vault.path(), &note);
    }

    #[test]
    fn nested_directory_create_rename_and_remove_keep_both_sides() {
        let vault = tempfile::tempdir().unwrap();
        let before = scan(vault.path()).unwrap();
        let old = vault.path().join("Old");
        fs::create_dir_all(old.join("Nested")).unwrap();
        fs::write(old.join("Nested/Note.md"), "note").unwrap();
        let current = scan(vault.path()).unwrap();
        let created = changes(&before, &current);
        assert_eq!(created.len(), 3);
        assert!(created.iter().all(|event| event.kind.is_create()));
        let new = vault.path().join("New");
        fs::rename(&old, &new).unwrap();
        let renamed = scan(vault.path()).unwrap();
        let events = changes(&current, &renamed);
        assert_eq!(
            events.iter().filter(|event| event.kind.is_create()).count(),
            3
        );
        assert_eq!(
            events.iter().filter(|event| event.kind.is_remove()).count(),
            3
        );
        fs::remove_dir_all(&new).unwrap();
        let events = changes(&renamed, &scan(vault.path()).unwrap());
        assert_eq!(events.len(), 3);
        assert!(events.iter().all(|event| event.kind.is_remove()));
    }

    #[test]
    fn hidden_resources_and_attachment_changes_are_excluded() {
        let vault = tempfile::tempdir().unwrap();
        for name in [
            ".helixnotes",
            ".hidden",
            "assets",
            "_attachments",
            "node_modules",
        ] {
            fs::create_dir(vault.path().join(name)).unwrap();
            fs::write(vault.path().join(name).join("Hidden.md"), "hidden").unwrap();
        }
        let before = scan(vault.path()).unwrap();
        assert_eq!(
            before.len(),
            1,
            "only the explicitly selected vault root should be present"
        );
        fs::write(vault.path().join("image.png"), "attachment").unwrap();
        fs::write(vault.path().join(".hidden.md"), "hidden").unwrap();
        assert!(changes(&before, &scan(vault.path()).unwrap()).is_empty());
    }

    #[test]
    fn note_symlinks_are_watched_without_following_directory_symlinks() {
        let vault = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let note = vault.path().join("Note.md");
        let alias = vault.path().join("Alias.md");
        fs::write(&note, "before").unwrap();
        fs::write(outside.path().join("Outside.md"), "outside").unwrap();
        symlink(&note, &alias).unwrap();
        symlink(outside.path(), vault.path().join("Directory.md")).unwrap();
        symlink(
            vault.path().join("Missing.md"),
            vault.path().join("Broken.md"),
        )
        .unwrap();
        let before = scan(vault.path()).unwrap();
        assert_eq!(before.len(), 3);
        assert!(before.contains_key(&alias));
        set_modified(&note, UNIX_EPOCH + Duration::from_secs(1000));
        let events = changes(&before, &scan(vault.path()).unwrap());
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|event| event.kind.is_modify()));
    }

    #[test]
    fn scan_errors_do_not_publish_partial_snapshots_and_recovery_detects_edits() {
        let vault = tempfile::tempdir().unwrap();
        let note = vault.path().join("Note.md");
        fs::write(&note, "before").unwrap();
        let (sender, receiver) = mpsc::channel();
        let watcher =
            PortalWatcher::start(vault.path().to_owned(), sender, Duration::from_millis(20))
                .unwrap();
        let broken = vault.path().join("Loop.md");
        symlink(&broken, &broken).unwrap();
        assert!(receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .is_err());
        set_modified(&note, UNIX_EPOCH + Duration::from_secs(1000));
        fs::remove_file(&broken).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let event = receiver
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
            if let Ok(event) = event {
                assert_eq!(event.paths.as_slice(), std::slice::from_ref(&note));
                assert!(event.kind.is_modify());
                break;
            }
        }
        drop(watcher);
    }

    #[test]
    fn initial_failure_is_synchronous_and_drop_wakes_a_long_poll() {
        let vault = tempfile::tempdir().unwrap();
        let (sender, _receiver) = mpsc::channel();
        assert!(PortalWatcher::start(
            vault.path().join("Missing"),
            sender,
            Duration::from_secs(60)
        )
        .is_err());
        let (sender, receiver) = mpsc::channel();
        let watcher =
            PortalWatcher::start(vault.path().to_owned(), sender, Duration::from_secs(60)).unwrap();
        drop(watcher);
        assert!(matches!(
            receiver.recv_timeout(Duration::from_secs(2)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
    }
}
