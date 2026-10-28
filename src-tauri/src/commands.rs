use crate::asset_scope;
use crate::search::SearchIndex;
use crate::state::AppState;
use crate::types::*;
use crate::vault::operations;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use tauri_plugin_fs::FsExt;

fn index_note_bg(search: Option<Arc<SearchIndex>>, path: &str) {
    if let Some(search) = search {
        if let Err(error) = search.queue_index_note(path) {
            log::error!("Failed to queue note indexing: {error}");
        }
    }
}

fn remove_note_bg(search: Option<Arc<SearchIndex>>, path: &str) {
    if let Some(search) = search {
        let p = path.to_string();
        std::thread::spawn(move || {
            if let Err(error) = search.remove_note(&p) {
                log::error!("Failed to remove note from search index: {error}");
            }
        });
    }
}

#[derive(Clone, serde::Serialize)]
struct PathMovedEvent<'a> {
    vault_path: &'a str,
    old_path: &'a str,
    new_path: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    old_raw: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    new_raw: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    old_title: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    new_title: Option<&'a str>,
}

fn emit_path_move<R: Runtime>(
    app: &AppHandle<R>,
    vault: &str,
    old: &str,
    new: &str,
    renamed: Option<&operations::RenamedNote>,
) {
    // Called before releasing note_save; watcher reconciliation takes that lock too.
    if let Err(error) = app.emit(
        "vault-path-moved",
        PathMovedEvent {
            vault_path: vault,
            old_path: old,
            new_path: new,
            old_raw: renamed.map(|note| note.old_raw.as_str()),
            new_raw: renamed.map(|note| note.new_raw.as_str()),
            old_title: renamed.map(|note| note.old_title.as_str()),
            new_title: renamed.map(|note| note.new_title.as_str()),
        },
    ) {
        log::error!("Could not notify note windows about a moved path: {error}");
    }
}

#[derive(Clone, serde::Serialize)]
struct LinksRewrittenEvent<'a> {
    vault_path: &'a str,
    path: &'a str,
    old_raw: &'a str,
    new_raw: &'a str,
    renames: &'a [operations::LinkRename],
}

// A rename also rewrites the notes that link to the renamed one. Keep their previous state in
// version history, and tell open editors so unsaved edits in such a note are not refused as a
// conflict with the disk.
fn announce_rewritten_links<R: Runtime>(
    app: &AppHandle<R>,
    vault: &str,
    links: &operations::LinkRewrite,
    max_versions: u32,
) {
    for note in &links.notes {
        let filename = Path::new(&note.path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let (meta, _) = crate::vault::frontmatter::parse_note(&note.old_raw, &filename);
        if !meta.id.is_empty() {
            crate::history::maybe_snapshot(vault, &meta.id, &note.old_raw, max_versions);
        }
        if let Err(error) = app.emit(
            "vault-links-rewritten",
            LinksRewrittenEvent {
                vault_path: vault,
                path: &note.path,
                old_raw: &note.old_raw,
                new_raw: &note.new_raw,
                renames: &links.renames,
            },
        ) {
            log::error!("Could not notify note windows about updated links: {error}");
        }
    }
}

// ── Vault Management ──

#[cfg(target_os = "ios")]
use tauri_plugin_ios_vault_access::{FolderSelection, IosVaultAccessExt};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalVaultResult {
    pub bookmark_id: String,
    pub path: String,
    pub name: String,
}

#[cfg(target_os = "ios")]
impl From<FolderSelection> for ExternalVaultResult {
    fn from(selection: FolderSelection) -> Self {
        Self {
            bookmark_id: selection.bookmark_id,
            path: selection.path,
            name: selection.name,
        }
    }
}

fn open_vault_path(
    app: AppHandle,
    state: &State<'_, AppState>,
    path: String,
    external: Option<(String, String)>,
) -> Result<std::sync::Arc<SearchIndex>, String> {
    operations::ensure_vault_structure(&path)?;

    // Stage the search index and watcher before replacing the active runtime.
    let search = state.search_index_for_vault(&path)?;

    asset_scope::allow_vault_assets(&app, Path::new(&path))?;
    state.ensure_vault_watcher(&app, &path)?;

    // Update config. External vaults use the bookmark as their stable identity;
    // the resolved path is refreshed whenever the vault opens.
    let mut search_slot = state.search_index.lock().map_err(|e| e.to_string())?;
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    let mut next = config.clone();
    let active_bookmark_id = external
        .as_ref()
        .map(|(bookmark_id, _)| bookmark_id.clone());
    if let Some((bookmark_id, name)) = external {
        if let Some(vault) = next
            .vaults
            .iter_mut()
            .find(|vault| vault.bookmark_id.as_deref() == Some(bookmark_id.as_str()))
        {
            vault.path.clone_from(&path);
            vault.name = name;
        } else {
            next.vaults.push(VaultConfig {
                path: path.clone(),
                name,
                bookmark_id: Some(bookmark_id),
                ..Default::default()
            });
        }
    } else if !next
        .vaults
        .iter()
        .any(|vault| vault.bookmark_id.is_none() && vault.path == path)
    {
        let name = std::path::Path::new(&path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        next.vaults.push(VaultConfig {
            path: path.clone(),
            name,
            ..Default::default()
        });
    }
    next.active_vault = Some(path);
    next.active_bookmark_id = active_bookmark_id;
    if let Err(error) = save_app_config(&next) {
        drop(config);
        drop(search_slot);
        let _ = state.retain_vault_watchers();
        return Err(error);
    }
    let old_search = search_slot.replace(search.clone());
    *config = next;
    drop(config);
    drop(search_slot);
    // Index disposal and watcher shutdown must not hold global state slots.
    state.retain_vault_watchers()?;
    drop(old_search);

    Ok(search)
}

#[tauri::command]
pub async fn open_vault(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<(), String> {
    let _transition = state.vault_transition.lock().await;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let search = open_vault_path(app.clone(), &state, path.clone(), None)?;
        #[cfg(target_os = "ios")]
        app.ios_vault_access().release_active()?;
        if let Err(error) = search.queue_vault_refresh(&path) {
            log::error!("Failed to queue opened vault search refresh: {error}");
        }
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn choose_external_vault(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<ExternalVaultResult>, String> {
    let _transition = state.vault_transition.lock().await;
    #[cfg(target_os = "ios")]
    {
        let picker_app = app.clone();
        let selection = tauri::async_runtime::spawn_blocking(move || {
            picker_app.ios_vault_access().choose_folder()
        })
        .await
        .map_err(|error| error.to_string())??;
        if selection.cancelled {
            return Ok(None);
        }
        let result = ExternalVaultResult::from(selection);
        tauri::async_runtime::spawn_blocking(move || {
            let state = app.state::<AppState>();
            let bookmark_was_registered = state
                .config
                .lock()
                .map_err(|error| error.to_string())?
                .vaults
                .iter()
                .any(|vault| vault.bookmark_id.as_deref() == Some(result.bookmark_id.as_str()));
            let search = match open_vault_path(
                app.clone(),
                &state,
                result.path.clone(),
                Some((result.bookmark_id.clone(), result.name.clone())),
            ) {
                Ok(search) => search,
                Err(error) => {
                    let rollback = if bookmark_was_registered {
                        app.ios_vault_access().rollback_staged()
                    } else {
                        app.ios_vault_access().forget_bookmark(&result.bookmark_id)
                    };
                    return Err(match rollback {
                        Ok(()) => error,
                        Err(rollback_error) => {
                            format!("{error} Bookmark rollback also failed: {rollback_error}")
                        }
                    });
                }
            };
            app.ios_vault_access().commit_staged()?;
            if let Err(error) = search.queue_vault_refresh(&result.path) {
                log::error!("Failed to queue opened vault search refresh: {error}");
            }
            Ok(Some(result))
        })
        .await
        .map_err(|error| error.to_string())?
    }
    #[cfg(not(target_os = "ios"))]
    {
        let _ = (&app, &state);
        Err("Files folders are only available on iOS.".to_string())
    }
}

#[tauri::command]
pub async fn restore_external_vault(
    app: AppHandle,
    state: State<'_, AppState>,
    bookmark_id: String,
) -> Result<ExternalVaultResult, String> {
    let _transition = state.vault_transition.lock().await;
    #[cfg(target_os = "ios")]
    {
        let resolver_app = app.clone();
        let selection = tauri::async_runtime::spawn_blocking(move || {
            resolver_app.ios_vault_access().resolve_folder(&bookmark_id)
        })
        .await
        .map_err(|error| error.to_string())??;
        let result = ExternalVaultResult::from(selection);
        tauri::async_runtime::spawn_blocking(move || {
            let state = app.state::<AppState>();
            let search = match open_vault_path(
                app.clone(),
                &state,
                result.path.clone(),
                Some((result.bookmark_id.clone(), result.name.clone())),
            ) {
                Ok(search) => search,
                Err(error) => {
                    return Err(match app.ios_vault_access().rollback_staged() {
                        Ok(()) => error,
                        Err(rollback_error) => {
                            format!("{error} Bookmark rollback also failed: {rollback_error}")
                        }
                    });
                }
            };
            app.ios_vault_access().commit_staged()?;
            if let Err(error) = search.queue_vault_refresh(&result.path) {
                log::error!("Failed to queue opened vault search refresh: {error}");
            }
            Ok(result)
        })
        .await
        .map_err(|error| error.to_string())?
    }
    #[cfg(not(target_os = "ios"))]
    {
        let _ = (&app, &state, &bookmark_id);
        Err("Files folders are only available on iOS.".to_string())
    }
}

#[tauri::command]
pub async fn remove_vault(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    bookmark_id: Option<String>,
) -> Result<(), String> {
    let _transition = state.vault_transition.lock().await;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let config_path = app_config_path()?;
        remove_vault_path(&app, &state, path, bookmark_id, &config_path)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn remove_vault_path<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    path: String,
    bookmark_id: Option<String>,
    config_path: &Path,
) -> Result<(), String> {
    let find = |config: &AppConfig| {
        config
            .vaults
            .iter()
            .find(|vault| match bookmark_id.as_deref() {
                Some(bookmark_id) => vault.bookmark_id.as_deref() == Some(bookmark_id),
                None => vault.bookmark_id.is_none() && vault.path == path,
            })
            .cloned()
    };
    // Asking a window for its address waits for the main thread, and the main thread may be
    // waiting for the locks taken below. Check the windows before holding any of them.
    #[cfg(desktop)]
    {
        // Only a bookmarked vault needs the list to learn its path.
        let listed = match bookmark_id {
            Some(_) => find(&*state.config.lock().map_err(|error| error.to_string())?)
                .map(|vault| vault.path),
            None => Some(path.clone()),
        };
        if let Some(vault_path) = listed {
            AppState::ensure_no_detached_windows(app, &vault_path)?;
        }
    }
    let mut search_slot = state
        .search_index
        .lock()
        .map_err(|error| error.to_string())?;
    let mut config = state.config.lock().map_err(|error| error.to_string())?;
    let target = find(&config).map(|vault| {
        let is_active = if let Some(bookmark_id) = vault.bookmark_id.as_deref() {
            config.active_bookmark_id.as_deref() == Some(bookmark_id)
        } else {
            config.active_bookmark_id.is_none()
                && config.active_vault.as_deref() == Some(vault.path.as_str())
        };
        (vault.path.clone(), vault.bookmark_id.clone(), is_active)
    });

    let Some((target_path, target_bookmark, is_active)) = target else {
        return Ok(());
    };
    let old = config.clone();
    let mut next = old.clone();
    if let Some(id) = target_bookmark.as_deref() {
        next.vaults
            .retain(|vault| vault.bookmark_id.as_deref() != Some(id));
    } else {
        next.vaults
            .retain(|vault| vault.bookmark_id.is_some() || vault.path != target_path);
    }
    if is_active {
        next.active_vault = None;
        next.active_bookmark_id = None;
    }
    save_app_config_to(&next, config_path)?;

    #[cfg(target_os = "ios")]
    if let Some(id) = target_bookmark.as_deref() {
        if let Err(error) = app.ios_vault_access().forget_bookmark(id) {
            return match save_app_config_to(&old, config_path) {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(format!(
                    "{error} Configuration rollback also failed: {rollback_error}"
                )),
            };
        }
    }
    #[cfg(not(target_os = "ios"))]
    let _ = (app, &target_bookmark);

    let old_search = if is_active { search_slot.take() } else { None };
    *config = next;
    drop(config);
    drop(search_slot);
    state.retain_vault_watchers()?;
    drop(old_search);

    Ok(())
}

#[tauri::command]
pub fn get_app_config(state: State<'_, AppState>) -> Result<AppConfig, String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;
    Ok(config.clone())
}

#[tauri::command]
pub fn get_pending_open_file(
    state: State<'_, AppState>,
) -> Result<Option<crate::state::PendingOpenFile>, String> {
    state.peek_pending_open_file()
}

#[tauri::command]
pub fn ack_pending_open_file(state: State<'_, AppState>, id: u64) -> Result<(), String> {
    if !state.ack_pending_open_file(id)? {
        return Err("File-open request is no longer pending".to_string());
    }
    Ok(())
}

#[cfg(desktop)]
#[tauri::command]
pub fn cancel_window_close(state: State<'_, AppState>) {
    state
        .quitting
        .store(false, std::sync::atomic::Ordering::Relaxed);
}

/// Finish a desktop close request only after the frontend has flushed its editor.
#[cfg(desktop)]
#[tauri::command]
pub fn complete_window_close<R: Runtime>(
    window: tauri::Window<R>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if state.should_hide_to_tray(
        window.label(),
        window.app_handle().tray_by_id("helixnotes").is_some(),
    )? {
        return window.hide().map_err(|error| error.to_string());
    }
    window.destroy().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_theme(state: State<'_, AppState>, theme: String) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    config.theme = theme;
    save_app_config(&config)?;
    Ok(())
}

#[tauri::command]
pub fn set_system_themes(
    state: State<'_, AppState>,
    light: String,
    dark: String,
) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    config.system_light_theme = light;
    config.system_dark_theme = dark;
    save_app_config(&config)?;
    Ok(())
}

#[tauri::command]
pub fn set_accent_color(state: State<'_, AppState>, color: String) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    config.accent_color = Some(color);
    save_app_config(&config)?;
    Ok(())
}

#[tauri::command]
pub fn save_custom_theme(
    state: State<'_, AppState>,
    theme: crate::types::CustomTheme,
) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    if let Some(pos) = config.custom_themes.iter().position(|t| t.id == theme.id) {
        config.custom_themes[pos] = theme;
    } else {
        config.custom_themes.push(theme);
    }
    save_app_config(&config)?;
    Ok(())
}

#[tauri::command]
pub fn delete_custom_theme(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    clear_custom_theme_references(&mut config, &id);
    save_app_config(&config)?;
    Ok(())
}

fn clear_custom_theme_references(config: &mut AppConfig, id: &str) {
    config.custom_themes.retain(|t| t.id != id);
    if config.theme == id {
        config.theme = "system".to_string();
    }
    if config.system_light_theme == id {
        config.system_light_theme = "light".to_string();
    }
    if config.system_dark_theme == id {
        config.system_dark_theme = "dark".to_string();
    }
}

#[cfg(test)]
mod custom_theme_reference_tests {
    use super::*;

    #[test]
    fn deleting_custom_theme_resets_system_pair_references() {
        let mut config = AppConfig {
            theme: "custom-work".to_string(),
            system_light_theme: "custom-work".to_string(),
            system_dark_theme: "custom-work".to_string(),
            ..Default::default()
        };

        clear_custom_theme_references(&mut config, "custom-work");

        assert_eq!(config.theme, "system");
        assert_eq!(config.system_light_theme, "light");
        assert_eq!(config.system_dark_theme, "dark");
    }
}

#[tauri::command]
pub fn export_custom_theme(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    path: String,
) -> Result<(), String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;
    let theme = config
        .custom_themes
        .iter()
        .find(|t| t.id == id)
        .ok_or_else(|| "Theme not found".to_string())?;
    let export = serde_json::json!({ "version": 1, "themes": [theme] });
    let data = serde_json::to_string_pretty(&export).map_err(|e| e.to_string())?;
    ensure_scoped_path(&app, Path::new(&path))?;
    std::fs::write(&path, data).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn import_custom_themes(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<Vec<crate::types::CustomTheme>, String> {
    ensure_scoped_path(&app, Path::new(&path))?;
    let data = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let parsed: serde_json::Value = serde_json::from_str(&data).map_err(|e| e.to_string())?;
    let themes: Vec<crate::types::CustomTheme> =
        serde_json::from_value(parsed["themes"].clone())
            .map_err(|e| format!("Invalid theme file: {}", e))?;
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    for theme in &themes {
        if let Some(pos) = config.custom_themes.iter().position(|t| t.id == theme.id) {
            config.custom_themes[pos] = theme.clone();
        } else {
            config.custom_themes.push(theme.clone());
        }
    }
    save_app_config(&config)?;
    Ok(themes)
}

#[tauri::command]
pub fn set_font_size(
    app: AppHandle,
    state: State<'_, AppState>,
    size: u32,
    preferred: Option<bool>,
) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    config.font_size = Some(size);
    // Only a choice made in Appearance becomes the size that resetting the zoom returns to.
    if preferred == Some(true) {
        config.preferred_font_size = Some(size);
    }
    save_app_config(&config)?;
    drop(config);

    use tauri::Emitter;
    app.emit("editor-font-size-changed", size)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_scroll_to_change_font_size(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    config.scroll_to_change_font_size = enabled;
    save_app_config(&config)?;
    drop(config);

    use tauri::Emitter;
    app.emit("scroll-to-change-font-size-changed", enabled)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_toolbar_layout(
    app: AppHandle,
    state: State<'_, AppState>,
    layout: Option<ToolbarLayout>,
) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    let previous = std::mem::replace(&mut config.toolbar_layout, layout);
    if let Err(error) = save_app_config(&config) {
        config.toolbar_layout = previous;
        return Err(error);
    }

    use tauri::Emitter;
    app.emit("toolbar-layout-changed", &config.toolbar_layout)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_font_family(state: State<'_, AppState>, family: String) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    config.font_family = Some(family);
    save_app_config(&config)?;
    Ok(())
}

#[tauri::command]
pub fn set_line_height(state: State<'_, AppState>, height: f64) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    config.line_height = Some(height);
    save_app_config(&config)?;
    Ok(())
}

#[tauri::command]
pub fn set_ui_scale(app: AppHandle, state: State<'_, AppState>, scale: f64) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    config.ui_scale = Some(scale);
    save_app_config(&config)?;
    drop(config);

    use tauri::Emitter;
    app.emit("ui-scale-changed", scale)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_content_width(state: State<'_, AppState>, width: Option<u32>) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    config.content_width = width;
    save_app_config(&config)?;
    Ok(())
}

// ── Notebooks ──

#[tauri::command]
pub async fn get_notebooks(state: State<'_, AppState>) -> Result<Vec<NotebookEntry>, String> {
    let vault_path = state
        .config
        .lock()
        .map_err(|error| error.to_string())?
        .active_vault
        .clone()
        .ok_or("No active vault")?;
    tauri::async_runtime::spawn_blocking(move || operations::scan_notebooks(&vault_path))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn count_root_notes(state: State<'_, AppState>) -> Result<usize, String> {
    let vault_path = state
        .config
        .lock()
        .map_err(|error| error.to_string())?
        .active_vault
        .clone()
        .ok_or("No active vault")?;
    tauri::async_runtime::spawn_blocking(move || operations::count_root_notes(&vault_path))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn create_notebook(
    state: State<'_, AppState>,
    vault_path: String,
    parent_relative: Option<String>,
    name: String,
) -> Result<NotebookEntry, String> {
    state.validate_vault(&vault_path)?;
    let _write_guard = state.note_save.lock().await;
    operations::create_notebook(&vault_path, parent_relative.as_deref(), &name)
}

#[tauri::command]
pub async fn rename_notebook<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    vault_path: String,
    path: String,
    new_name: String,
) -> Result<String, String> {
    let search = state.vault_snapshot(&vault_path)?;
    let max_versions = state
        .config
        .lock()
        .map_err(|error| error.to_string())?
        .max_versions_per_note;
    let _write_guard = state.note_save.lock().await;
    let (renamed, links) = operations::rename_notebook(&vault_path, &path, &new_name)?;
    emit_path_move(&app, &vault_path, &path, &renamed, None);
    announce_rewritten_links(&app, &vault_path, &links, max_versions);
    if let Some(search) = search {
        if let Err(error) = search.queue_vault_refresh(&vault_path) {
            log::error!("Failed to queue renamed notebook search refresh: {error}");
        }
    }
    Ok(renamed)
}

#[tauri::command]
pub async fn move_notebook<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    vault_path: String,
    notebook_path: String,
    dest_parent: String,
) -> Result<String, String> {
    let search = state.vault_snapshot(&vault_path)?;
    let max_versions = state
        .config
        .lock()
        .map_err(|error| error.to_string())?
        .max_versions_per_note;
    let _write_guard = state.note_save.lock().await;
    let vault_path = vault_path.as_str();

    let (new_full_path, links) =
        operations::move_notebook(vault_path, &notebook_path, &dest_parent)?;
    emit_path_move(&app, vault_path, &notebook_path, &new_full_path, None);
    announce_rewritten_links(&app, vault_path, &links, max_versions);

    // Reconcile renamed paths without blocking the move on a full rebuild.
    if let Some(search) = search {
        if let Err(error) = search.queue_vault_refresh(vault_path) {
            log::error!("Failed to queue moved notebook search refresh: {error}");
        }
    }

    Ok(new_full_path)
}

#[tauri::command]
pub async fn delete_notebook(
    state: State<'_, AppState>,
    vault_path: String,
    path: String,
) -> Result<(), String> {
    let search = state.vault_snapshot(&vault_path)?;
    let _write_guard = state.note_save.lock().await;
    operations::delete_notebook(&vault_path, &path)?;
    if let Some(search) = search {
        if let Err(error) = search.queue_vault_refresh(&vault_path) {
            log::error!("Failed to queue deleted notebook search refresh: {error}");
        }
    }
    Ok(())
}

// ── Notes ──

#[tauri::command]
pub async fn get_notes(
    state: State<'_, AppState>,
    notebook_path: Option<String>,
) -> Result<Vec<NoteEntry>, String> {
    let vault_path = state
        .config
        .lock()
        .map_err(|error| error.to_string())?
        .active_vault
        .clone()
        .ok_or("No active vault")?;
    tauri::async_runtime::spawn_blocking(move || {
        operations::scan_notes(&vault_path, notebook_path.as_deref())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn read_note<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    vault_path: String,
    path: String,
) -> Result<NoteContent, String> {
    state.validate_vault(&vault_path)?;
    tauri::async_runtime::spawn_blocking(move || {
        let vault = std::fs::canonicalize(&vault_path).map_err(|error| error.to_string())?;
        let target = std::fs::canonicalize(&path).map_err(|error| error.to_string())?;
        if target.starts_with(vault) {
            // Keep vault/trash validation, even if app metadata was explicitly selected.
            return operations::read_note(&vault_path, &path);
        }
        // OS file-open requests, dialogs and drops grant this scope. Never grant it here.
        ensure_scoped_path(&app, &target)?;
        if !target.is_file()
            || target.extension().and_then(|extension| extension.to_str()) != Some("md")
        {
            return Err("Note path must point to a Markdown file".to_string());
        }
        operations::read_note_content(&target, &path)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn save_note(
    state: State<'_, AppState>,
    vault_path: String,
    path: String,
    meta: NoteMeta,
    body: String,
    expected_raw: String,
) -> Result<NoteContent, String> {
    let search = state.vault_snapshot(&vault_path)?;
    let max_versions = state
        .config
        .lock()
        .map_err(|error| error.to_string())?
        .max_versions_per_note;
    // Capture the origin before queueing. Waiting must not retarget the write
    // when the main window switches vaults or the calling window closes.
    let save_guard = state.note_save.clone().lock_owned().await;

    tauri::async_runtime::spawn_blocking(move || {
        // Keep ordering even if the calling window closes while its save runs.
        let _save_guard = save_guard;
        if let Some(saved) = operations::save_note(&vault_path, &path, &meta, &body, &expected_raw)?
        {
            crate::history::maybe_snapshot(
                &vault_path,
                &saved.id,
                &saved.previous_raw,
                max_versions,
            );
            index_note_bg(search, &path);
        }
        operations::read_vault_note(&vault_path, &path)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn import_note<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    vault_path: String,
    source_path: String,
    notebook_relative: Option<String>,
    expected_raw: String,
    references: Vec<operations::ExternalNoteReference>,
) -> Result<NoteContent, String> {
    let search = state.vault_snapshot(&vault_path)?;
    let maintenance = state.vault_maintenance.clone().lock_owned().await;
    let writes = state.note_save.clone().lock_owned().await;
    tauri::async_runtime::spawn_blocking(move || {
        let (_maintenance, _writes) = (maintenance, writes);
        let source_root = asset_scope::external_note_root(&app, Path::new(&source_path))?;
        let note = operations::import_note(
            &vault_path,
            &source_path,
            &source_root,
            notebook_relative.as_deref(),
            &expected_raw,
            references,
            |path| ensure_scoped_path(&app, path),
        )?;
        index_note_bg(search, &note.path);
        let event = crate::vault::watcher::changed_entry(
            PathBuf::from(&note.path),
            "create",
            Path::new(&vault_path),
        );
        let _ = app.emit("file-changed", event);
        Ok(note)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn duplicate_note(
    state: State<'_, AppState>,
    vault_path: String,
    path: String,
) -> Result<crate::types::NoteEntry, String> {
    let search = state.vault_snapshot(&vault_path)?;
    let _write_guard = state.note_save.lock().await;
    let entry = operations::duplicate_note(&path, &vault_path)?;
    index_note_bg(search, &entry.path);
    Ok(entry)
}

#[tauri::command]
pub async fn create_note(
    state: State<'_, AppState>,
    vault_path: String,
    notebook_relative: Option<String>,
    title: String,
) -> Result<NoteEntry, String> {
    let search = state.vault_snapshot(&vault_path)?;
    let _write_guard = state.note_save.lock().await;
    let entry = operations::create_note(&vault_path, notebook_relative.as_deref(), &title)?;

    // Index new note (background to avoid blocking on FUSE fsync)
    index_note_bg(search, &entry.path);

    Ok(entry)
}

#[tauri::command]
pub async fn create_daily_note(
    state: State<'_, AppState>,
    vault_path: String,
    date: Option<String>,
) -> Result<NoteEntry, String> {
    let search = state.vault_snapshot(&vault_path)?;
    let title_format = state
        .config
        .lock()
        .map_err(|error| error.to_string())?
        .daily_title_format
        .clone();
    let _write_guard = state.note_save.lock().await;
    let entry = operations::create_daily_note(&vault_path, date.as_deref(), &title_format)?;

    index_note_bg(search, &entry.path);

    Ok(entry)
}

#[tauri::command]
pub async fn rename_note<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    vault_path: String,
    path: String,
    new_title: String,
) -> Result<String, String> {
    let search = state.vault_snapshot(&vault_path)?;
    let max_versions = state
        .config
        .lock()
        .map_err(|error| error.to_string())?
        .max_versions_per_note;
    let _write_guard = state.note_save.lock().await;
    let renamed = operations::rename_note(&path, &new_title, &vault_path)?;
    emit_path_move(&app, &vault_path, &path, &renamed.path, Some(&renamed));
    announce_rewritten_links(&app, &vault_path, &renamed.links, max_versions);
    if let Some(search) = search {
        if let Err(error) = search.queue_vault_refresh(&vault_path) {
            log::error!("Failed to queue renamed note search refresh: {error}");
        }
    }
    Ok(renamed.path)
}

#[tauri::command]
pub async fn delete_note(
    state: State<'_, AppState>,
    vault_path: String,
    path: String,
) -> Result<(), String> {
    let search = state.vault_snapshot(&vault_path)?;
    let _write_guard = state.note_save.lock().await;
    operations::delete_note(&vault_path, &path)?;

    // Remove from index (background to avoid blocking on FUSE fsync)
    remove_note_bg(search, &path);

    Ok(())
}

#[tauri::command]
pub async fn move_note<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    vault_path: String,
    note_path: String,
    dest_notebook: String,
) -> Result<String, String> {
    let search = state.vault_snapshot(&vault_path)?;
    let max_versions = state
        .config
        .lock()
        .map_err(|error| error.to_string())?
        .max_versions_per_note;
    let _write_guard = state.note_save.lock().await;
    let vault_path = vault_path.as_str();

    let (new_full_path, links) = operations::move_note(vault_path, &note_path, &dest_notebook)?;
    emit_path_move(&app, vault_path, &note_path, &new_full_path, None);
    announce_rewritten_links(&app, vault_path, &links, max_versions);

    index_note_bg(search.clone(), &note_path);
    index_note_bg(search.clone(), &new_full_path);
    for note in &links.notes {
        index_note_bg(search.clone(), &note.path);
    }

    Ok(new_full_path)
}

// ── Tags ──

#[tauri::command]
pub async fn get_all_tags(state: State<'_, AppState>) -> Result<Vec<(String, usize)>, String> {
    let vault_path = state
        .config
        .lock()
        .map_err(|error| error.to_string())?
        .active_vault
        .clone()
        .ok_or("No active vault")?;
    tauri::async_runtime::spawn_blocking(move || operations::get_all_tags(&vault_path))
        .await
        .map_err(|error| error.to_string())?
}

// ── Wiki-links ──

#[tauri::command]
pub async fn get_all_note_titles(
    state: State<'_, AppState>,
    vault_path: String,
) -> Result<Vec<NoteTitleEntry>, String> {
    state.validate_vault(&vault_path)?;
    tauri::async_runtime::spawn_blocking(move || scan_all_note_titles(&vault_path))
        .await
        .map_err(|error| error.to_string())?
}

fn scan_all_note_titles(vault_path: &str) -> Result<Vec<NoteTitleEntry>, String> {
    use rayon::prelude::*;

    let vault = std::path::Path::new(vault_path);
    let hn_dir = operations::helixnotes_dir(vault_path);

    let paths: Vec<PathBuf> = walkdir::WalkDir::new(vault)
        .into_iter()
        // Cross-platform exclusion (string "/.helixnotes/" checks miss Windows
        // backslash paths, which leaked .helixnotes/history snapshots in as dupes).
        .filter_entry(|e| {
            e.depth() == 0 || (!operations::is_hidden(e.path()) && !e.path().starts_with(&hn_dir))
        })
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().is_file() && e.path().extension().and_then(|x| x.to_str()) == Some("md")
        })
        .map(|e| e.into_path())
        .collect();

    // Only the frontmatter is needed, so read the note head in parallel instead of whole files.
    Ok(paths
        .par_iter()
        .map(|path| {
            // Read frontmatter title, fallback to filename
            let title = operations::read_note_head(path)
                .ok()
                .and_then(|head| crate::vault::frontmatter::extract_title(&head))
                .unwrap_or_else(|| {
                    path.file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string()
                });

            // Store vault-relative path with forward slashes for cross-platform portability.
            // Absolute paths (e.g. C:\Users\...) break when the vault is synced to another device.
            let rel = path
                .strip_prefix(vault)
                .map(|r| r.to_string_lossy().replace('\\', "/").to_string())
                .unwrap_or_else(|_| path.to_string_lossy().to_string());

            NoteTitleEntry { title, path: rel }
        })
        .collect())
}

#[tauri::command]
pub async fn get_note_switcher_titles(
    state: State<'_, AppState>,
    recent_paths: Vec<String>,
) -> Result<Vec<NoteTitleEntry>, String> {
    let vault_path = state
        .config
        .lock()
        .map_err(|error| error.to_string())?
        .active_vault
        .clone()
        .ok_or_else(|| "No active vault".to_string())?;

    tauri::async_runtime::spawn_blocking(move || {
        operations::get_note_switcher_titles(&vault_path, &recent_paths)
    })
    .await
    .map_err(|error| error.to_string())?
}

// ── Graph ──

#[tauri::command]
pub async fn get_graph_data(
    state: State<'_, AppState>,
    vault_path: String,
) -> Result<crate::types::GraphData, String> {
    state.validate_vault(&vault_path)?;
    tauri::async_runtime::spawn_blocking(move || scan_graph_data(&vault_path))
        .await
        .map_err(|error| error.to_string())?
}

/// Wiki-link targets in `body`, in order, with `|alias`, `#heading` and `^block` removed and
/// lowercased (inline scan, no regex).
fn extract_wiki_links(body: &str) -> Vec<String> {
    let mut links = Vec::new();
    let bytes = body.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i + 1 < len {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            i += 2;
            let start = i;
            while i + 1 < len && !(bytes[i] == b']' && bytes[i + 1] == b']') {
                i += 1;
            }
            if i + 1 < len {
                let link_raw = &body[start..i];
                let link = link_raw.split('|').next().unwrap_or(link_raw);
                // In a table cell the alias separator is written `\|`.
                let link = link.strip_suffix('\\').unwrap_or(link);
                let link = link.split('#').next().unwrap_or(link);
                let link = link.split('^').next().unwrap_or(link);
                links.push(link.trim().to_lowercase());
                i += 2;
            }
        } else {
            i += 1;
        }
    }
    links
}

#[cfg(test)]
mod graph_link_tests {
    use super::extract_wiki_links;

    #[test]
    fn aliased_links_in_a_table_cell_are_found() {
        let body =
            "| a |\n| --- |\n| [[Home\\|start]] and [[Other]] and [[Deep/Note#part\\|x]] |\n";
        assert_eq!(extract_wiki_links(body), ["home", "other", "deep/note"]);
        assert_eq!(extract_wiki_links("[[Home|start]]"), ["home"]);
    }
}

fn scan_graph_data(vault_path: &str) -> Result<crate::types::GraphData, String> {
    use rayon::prelude::*;
    use std::collections::{HashMap, HashSet};

    let vault = std::path::Path::new(vault_path);
    let hn_dir = operations::helixnotes_dir(vault_path);

    // Collect every note path first. Cross-platform exclusion (string "/.helixnotes/" checks miss
    // Windows backslash paths; is_hidden also covers .stversions/.stfolder/.trash/.git).
    let candidates: Vec<PathBuf> = walkdir::WalkDir::new(vault)
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0 || (!operations::is_hidden(e.path()) && !e.path().starts_with(&hn_dir))
        })
        .filter_map(|e| e.ok())
        .filter(|entry| {
            let path = entry.path();
            path.is_file()
                && path.extension().and_then(|e| e.to_str()) == Some("md")
                // Skip Syncthing conflict files
                && !path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|name| name.contains(".sync-conflict-"))
        })
        .map(|entry| entry.into_path())
        .collect();

    // Deduplicate by canonical path (handles symlinks), keeping the first of each in walk order.
    let canonical: Vec<String> = candidates
        .par_iter()
        .map(|path| {
            path.canonicalize()
                .unwrap_or_else(|_| path.clone())
                .to_string_lossy()
                .to_string()
        })
        .collect();
    let mut seen_paths: HashSet<String> = HashSet::new();
    let paths: Vec<&PathBuf> = candidates
        .iter()
        .zip(canonical)
        .filter_map(|(path, canonical)| seen_paths.insert(canonical).then_some(path))
        .collect();

    // Read each note once, in parallel, and keep only its title and links so note bodies
    // never accumulate in memory.
    let scanned: Vec<(String, Vec<String>)> = paths
        .par_iter()
        .map(|path| {
            let raw = std::fs::read_to_string(path).unwrap_or_default();
            // Fast title extraction: scan for "title: " line in frontmatter without full YAML parse
            let title = extract_title_fast(&raw).unwrap_or_else(|| {
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            });
            (title, extract_wiki_links(&raw))
        })
        .collect();

    // Pass 1: collect ALL notes as nodes (no title deduplication - every note must appear)
    let mut graph_nodes = Vec::with_capacity(paths.len());
    // title → list of node indices (one title can map to multiple notes in different folders)
    let mut title_to_idxs: HashMap<String, Vec<usize>> = HashMap::new();
    // vault-relative path without extension → idx (for [[subfolder/note]] style links)
    let mut relpath_to_idx: HashMap<String, usize> = HashMap::new();
    let mut links_by_source: Vec<Vec<String>> = Vec::with_capacity(paths.len());

    for (path, (title, links)) in paths.iter().zip(scanned) {
        let idx = graph_nodes.len();
        title_to_idxs
            .entry(title.to_lowercase())
            .or_default()
            .push(idx);

        // Also index by vault-relative path without extension (e.g. "subfolder/note name")
        if let Ok(rel) = path.strip_prefix(vault) {
            let rel_no_ext = rel.with_extension("");
            // Normalize Windows backslashes so [[folder/note]] links resolve cross-platform.
            let rel_lower = rel_no_ext
                .to_string_lossy()
                .replace('\\', "/")
                .to_lowercase();
            relpath_to_idx.entry(rel_lower).or_insert(idx);
        }

        graph_nodes.push(crate::types::GraphNode {
            title,
            path: path.to_string_lossy().to_string(),
        });
        links_by_source.push(links);
    }

    // Pass 2: resolve wiki-links to edges
    // edge_map: (src, tgt) → index in edges vec, used to detect reverse links
    let mut edges: Vec<crate::types::GraphEdge> = Vec::new();
    let mut edge_map: HashMap<(usize, usize), usize> = HashMap::new();

    let add_edge = |edges: &mut Vec<crate::types::GraphEdge>,
                    edge_map: &mut HashMap<(usize, usize), usize>,
                    src: usize,
                    tgt: usize| {
        if src == tgt {
            return;
        }
        if edge_map.contains_key(&(src, tgt)) {
            return;
        } // exact duplicate
        if let Some(&rev_idx) = edge_map.get(&(tgt, src)) {
            // Reverse direction already exists - mark it as bidirectional
            edges[rev_idx].bidirectional = true;
        } else {
            let idx = edges.len();
            edge_map.insert((src, tgt), idx);
            edges.push(crate::types::GraphEdge {
                source: src,
                target: tgt,
                bidirectional: false,
            });
        }
    };

    for (source_idx, links) in links_by_source.iter().enumerate() {
        for link in links {
            // 1. Try vault-relative path match (e.g. "arkhost/note name")
            if let Some(&target_idx) = relpath_to_idx.get(link) {
                add_edge(&mut edges, &mut edge_map, source_idx, target_idx);
            } else if let Some(targets) = title_to_idxs.get(link) {
                // 2. Title match - connect to all notes with this title
                for &target_idx in targets {
                    add_edge(&mut edges, &mut edge_map, source_idx, target_idx);
                }
            } else if link.contains('/') {
                // 3. Path-based ref: try last segment as title fallback
                if let Some(seg) = link.rsplit('/').next() {
                    if let Some(targets) = title_to_idxs.get(seg) {
                        for &target_idx in targets {
                            add_edge(&mut edges, &mut edge_map, source_idx, target_idx);
                        }
                    }
                }
            }
        }
    }

    Ok(crate::types::GraphData {
        nodes: graph_nodes,
        edges,
    })
}

/// Fast title extraction from frontmatter without full YAML parsing.
/// Scans for `title: ...` line within `---` fences.
fn extract_title_fast(raw: &str) -> Option<String> {
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }
    // Find the closing ---
    let after_open = &trimmed[3..];
    let end = after_open.find("\n---")?;
    let frontmatter = &after_open[..end];
    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(title) = line.strip_prefix("title:") {
            let val = title.trim();
            // Strip surrounding quotes
            if (val.starts_with('"') && val.ends_with('"'))
                || (val.starts_with('\'') && val.ends_with('\''))
            {
                return Some(val[1..val.len() - 1].to_string());
            }
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

// ── Tasks ──

#[tauri::command]
pub async fn get_tasks(
    state: State<'_, AppState>,
    note_path: Option<String>,
) -> Result<Vec<TaskItem>, String> {
    let vault_path = state
        .config
        .lock()
        .map_err(|error| error.to_string())?
        .active_vault
        .clone()
        .ok_or("No active vault")?;
    tauri::async_runtime::spawn_blocking(move || scan_tasks(&vault_path, note_path.as_deref()))
        .await
        .map_err(|error| error.to_string())?
}

fn scan_tasks(vault_path: &str, note_path: Option<&str>) -> Result<Vec<TaskItem>, String> {
    use rayon::prelude::*;

    if let Some(path) = note_path {
        let note = operations::read_vault_note(vault_path, path)?;
        return Ok(parse_note_tasks(path, &note.meta.title, &note.content));
    }

    let vault = Path::new(vault_path);
    let hn_dir = operations::helixnotes_dir(vault_path);
    let paths: Vec<PathBuf> = walkdir::WalkDir::new(vault)
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0
                || (!operations::is_hidden(entry.path()) && !entry.path().starts_with(&hn_dir))
        })
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry.file_type().is_file()
                && entry
                    .path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    == Some("md")
        })
        .map(|entry| entry.into_path())
        .collect();

    Ok(paths
        .par_iter()
        .flat_map(|path| {
            let Ok(raw) = std::fs::read_to_string(path) else {
                return Vec::new();
            };
            let filename = path.file_name().unwrap_or_default().to_string_lossy();
            let (meta, body) = crate::vault::frontmatter::parse_note(&raw, &filename);
            parse_note_tasks(&path.to_string_lossy(), &meta.title, &body)
        })
        .collect())
}

fn parse_note_tasks(note_path: &str, note_title: &str, body: &str) -> Vec<TaskItem> {
    use std::sync::LazyLock;

    static TASK_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"^\s*[-*]\s\[([ xX])\]\s+(.+?)\s*$").unwrap());
    static DUE_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\bdue:(\d{4}-\d{2}-\d{2})\b").unwrap());
    static PRIORITY_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?i)(?:^|\s)!(high|medium|med|low)\b").unwrap());

    let mut tasks = Vec::new();
    for (line_number, line) in body.lines().enumerate() {
        let Some(caps) = TASK_RE.captures(line) else {
            continue;
        };
        let completed = !caps[1].trim().is_empty();
        let content = &caps[2];
        let due = DUE_RE
            .captures(content)
            .map(|captures| captures[1].to_string());
        let priority = PRIORITY_RE.captures(content).map(|captures| {
            let priority = captures[1].to_lowercase();
            if priority == "medium" {
                "med".to_string()
            } else {
                priority
            }
        });
        let text = DUE_RE.replace_all(content, "");
        let text = PRIORITY_RE.replace_all(&text, " ");
        tasks.push(TaskItem {
            note_path: note_path.to_string(),
            note_title: note_title.to_string(),
            line: line_number,
            raw_line: line.to_string(),
            text: text.split_whitespace().collect::<Vec<_>>().join(" "),
            completed,
            due,
            priority,
        });
    }
    tasks
}

#[cfg(test)]
mod note_title_scan_tests {
    use super::scan_all_note_titles;
    use std::fs;

    #[test]
    fn titles_come_from_complete_frontmatter_with_filename_fallback_and_relative_paths() {
        let parent = tempfile::tempdir().unwrap();
        let vault = parent.path().join("vault");
        fs::create_dir(&vault).unwrap();
        let tags = (0..200)
            .map(|index| format!("tag-{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        fs::create_dir_all(vault.join("Notebook/.helixnotes")).unwrap();
        fs::create_dir_all(vault.join(".helixnotes/history")).unwrap();
        fs::write(
            vault.join("Notebook/Long.md"),
            format!("---\ntags: [{tags}]\ntitle: Long Title\n---\n# Long Title\n"),
        )
        .unwrap();
        fs::write(vault.join("Plain.md"), "# No frontmatter\n").unwrap();
        fs::write(
            vault.join(".helixnotes/history/Old.md"),
            "---\ntitle: Old\n---\n",
        )
        .unwrap();
        fs::write(vault.join("Other.txt"), "---\ntitle: Skip\n---\n").unwrap();

        let mut titles = scan_all_note_titles(vault.to_str().unwrap()).unwrap();
        titles.sort_by(|a, b| a.path.cmp(&b.path));
        let found: Vec<_> = titles
            .iter()
            .map(|t| (t.path.as_str(), t.title.as_str()))
            .collect();
        assert_eq!(
            found,
            [("Notebook/Long.md", "Long Title"), ("Plain.md", "Plain")]
        );
    }
}

#[cfg(test)]
mod graph_scan_tests {
    use super::scan_graph_data;
    use std::fs;

    #[test]
    fn graph_links_resolve_by_path_and_title_and_mark_reverse_links() {
        let parent = tempfile::tempdir().unwrap();
        let vault = parent.path().join("vault");
        fs::create_dir_all(vault.join("folder")).unwrap();
        fs::create_dir_all(vault.join("other")).unwrap();
        fs::create_dir_all(vault.join(".helixnotes")).unwrap();
        let write = |path: &str, text: &str| fs::write(vault.join(path), text).unwrap();
        write("A.md", "---\ntitle: \"Alpha\"\n---\n[[Beta]] [[folder/C|alias]] [[Twin#Head]] [[missing]] [[unterminated");
        write(
            "B.md",
            "---\ntitle: Beta\n---\nback to [[alpha]] and [[Beta]] and [[nested/ghost/Beta]]",
        );
        write("folder/C.md", "---\ntitle: Gamma\n---\n[[A^block]]");
        write("folder/Twin.md", "---\ntitle: Twin\n---\n");
        write("other/Twin.md", "---\ntitle: Twin\n---\n");
        write("A.sync-conflict-20260101.md", "[[Beta]]");
        write(".helixnotes/Hidden.md", "[[Beta]]");
        write("ignored.txt", "[[Beta]]");

        let graph = scan_graph_data(vault.to_str().unwrap()).unwrap();
        let name = |index: usize| {
            let node = &graph.nodes[index];
            let relative = node.path.strip_prefix(vault.to_str().unwrap()).unwrap();
            format!(
                "{}|{}",
                relative.trim_start_matches(['/', '\\']).replace('\\', "/"),
                node.title
            )
        };
        let mut nodes: Vec<_> = (0..graph.nodes.len()).map(name).collect();
        nodes.sort();
        assert_eq!(
            nodes,
            [
                "A.md|Alpha",
                "B.md|Beta",
                "folder/C.md|Gamma",
                "folder/Twin.md|Twin",
                "other/Twin.md|Twin"
            ]
        );
        let mut edges: Vec<_> = graph
            .edges
            .iter()
            .map(|edge| {
                let mut pair = [name(edge.source), name(edge.target)];
                // Which side of a two-way link is stored first follows directory order.
                if edge.bidirectional {
                    pair.sort();
                }
                format!("{}>{}:{}", pair[0], pair[1], edge.bidirectional)
            })
            .collect();
        edges.sort();
        assert_eq!(
            edges,
            [
                "A.md|Alpha>B.md|Beta:true",
                "A.md|Alpha>folder/C.md|Gamma:true",
                "A.md|Alpha>folder/Twin.md|Twin:false",
                "A.md|Alpha>other/Twin.md|Twin:false",
            ]
        );
    }
}

#[cfg(test)]
mod dot_named_vault_tests {
    use super::{scan_all_note_titles, scan_graph_data, scan_tasks};
    use std::fs;

    #[test]
    fn vault_scans_do_not_treat_a_dot_named_vault_folder_as_hidden() {
        // tempdir() names its folder ".tmpXXXX", like a vault kept in ~/.notes.
        let vault = tempfile::tempdir().unwrap();
        assert!(vault
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with('.'));
        let vault_path = vault.path().to_str().unwrap();
        fs::write(
            vault.path().join("A.md"),
            "---\ntitle: Alpha\n---\n[[Beta]]\n- [ ] Task\n",
        )
        .unwrap();
        fs::write(
            vault.path().join("B.md"),
            "---\ntitle: Beta\n---\n[[Alpha]]\n",
        )
        .unwrap();

        assert_eq!(scan_all_note_titles(vault_path).unwrap().len(), 2);
        let graph = scan_graph_data(vault_path).unwrap();
        assert_eq!((graph.nodes.len(), graph.edges.len()), (2, 1));
        assert_eq!(scan_tasks(vault_path, None).unwrap().len(), 1);
    }
}

#[cfg(test)]
mod task_tests {
    use super::scan_tasks;

    #[test]
    fn targeted_task_scan_limits_results_and_keeps_task_metadata() {
        let vault = tempfile::tempdir().unwrap();
        let selected = vault.path().join("Selected.md");
        std::fs::write(
            &selected,
            "---\ntitle: Selected title\n---\n# Selected title\n\n- [x] Ship !medium due:2026-09-30\n",
        )
        .unwrap();
        std::fs::write(vault.path().join("Other.md"), "- [ ] Unrelated task\n").unwrap();

        let tasks = scan_tasks(
            &vault.path().to_string_lossy(),
            Some(&selected.to_string_lossy()),
        )
        .unwrap();

        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].note_path, selected.to_string_lossy());
        assert_eq!(tasks[0].note_title, "Selected title");
        assert_eq!(tasks[0].line, 2);
        assert_eq!(tasks[0].text, "Ship");
        assert_eq!(tasks[0].priority.as_deref(), Some("med"));
        assert_eq!(tasks[0].due.as_deref(), Some("2026-09-30"));
        assert!(tasks[0].completed);

        std::fs::write(&selected, "No tasks remain.\n").unwrap();
        assert!(scan_tasks(
            &vault.path().to_string_lossy(),
            Some(&selected.to_string_lossy()),
        )
        .unwrap()
        .is_empty());
    }

    #[test]
    fn targeted_task_scan_rejects_notes_outside_the_active_vault() {
        let root = tempfile::tempdir().unwrap();
        let vault = root.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let outside = root.path().join("Outside.md");
        std::fs::write(&outside, "- [ ] Outside task\n").unwrap();

        assert!(scan_tasks(&vault.to_string_lossy(), Some(&outside.to_string_lossy())).is_err());
    }
}

fn toggle_checkbox_line(line: &str, done: bool) -> String {
    let mut s = line.to_string();
    if done {
        if let Some(pos) = s.find("[ ]") {
            s.replace_range(pos..pos + 3, "[x]");
        }
    } else {
        for marker in ["[x]", "[X]"] {
            if let Some(pos) = s.find(marker) {
                s.replace_range(pos..pos + 3, "[ ]");
                break;
            }
        }
    }
    s
}

#[tauri::command]
pub async fn set_task_done(
    state: State<'_, AppState>,
    vault_path: String,
    note_path: String,
    line: usize,
    raw_line: String,
    done: bool,
) -> Result<(), String> {
    let search = state.vault_snapshot(&vault_path)?;
    let _write_guard = state.note_save.lock().await;
    let NoteContent {
        meta,
        content: body,
        raw,
        ..
    } = operations::read_vault_note(&vault_path, &note_path)?;
    let mut lines: Vec<String> = body.lines().map(|l| l.to_string()).collect();
    // Verify the expected line; if the note drifted, fall back to the first exact match.
    let idx = if lines.get(line).map(|l| *l == raw_line).unwrap_or(false) {
        line
    } else {
        lines
            .iter()
            .position(|l| *l == raw_line)
            .ok_or("Task line not found (note changed)")?
    };

    let toggled = toggle_checkbox_line(&lines[idx], done);
    if toggled == lines[idx] {
        return Ok(()); // already in the desired state
    }
    lines[idx] = toggled;
    let mut new_body = lines.join("\n");
    if body.ends_with('\n') && !new_body.ends_with('\n') {
        new_body.push('\n');
    }
    if operations::save_note(&vault_path, &note_path, &meta, &new_body, &raw)?.is_some() {
        index_note_bg(search, &note_path);
    }
    Ok(())
}

fn set_priority_on_line(line: &str, priority: Option<&str>) -> String {
    let prio_re = regex::Regex::new(r"(?i)(?:^|\s)!(?:high|medium|med|low)\b").unwrap();
    let stripped = prio_re.replace_all(line, "").to_string();
    let stripped = stripped.trim_end();
    match priority {
        Some(p) => format!("{} !{}", stripped, p),
        None => stripped.to_string(),
    }
}

#[tauri::command]
pub async fn set_task_priority(
    state: State<'_, AppState>,
    vault_path: String,
    note_path: String,
    line: usize,
    raw_line: String,
    priority: Option<String>,
) -> Result<(), String> {
    // Normalize/validate priority ("medium" -> "med"); None clears it.
    let prio = match priority.as_deref() {
        None | Some("") | Some("none") => None,
        Some("high") => Some("high"),
        Some("med") | Some("medium") => Some("med"),
        Some("low") => Some("low"),
        Some(_) => return Err("Invalid priority".to_string()),
    };

    let search = state.vault_snapshot(&vault_path)?;
    let _write_guard = state.note_save.lock().await;
    let NoteContent {
        meta,
        content: body,
        raw,
        ..
    } = operations::read_vault_note(&vault_path, &note_path)?;
    let mut lines: Vec<String> = body.lines().map(|l| l.to_string()).collect();
    let idx = if lines.get(line).map(|l| *l == raw_line).unwrap_or(false) {
        line
    } else {
        lines
            .iter()
            .position(|l| *l == raw_line)
            .ok_or("Task line not found (note changed)")?
    };

    let updated = set_priority_on_line(&lines[idx], prio);
    if updated == lines[idx] {
        return Ok(());
    }
    lines[idx] = updated;
    let mut new_body = lines.join("\n");
    if body.ends_with('\n') && !new_body.ends_with('\n') {
        new_body.push('\n');
    }
    if operations::save_note(&vault_path, &note_path, &meta, &new_body, &raw)?.is_some() {
        index_note_bg(search, &note_path);
    }
    Ok(())
}

fn set_due_on_line(line: &str, due: Option<&str>) -> String {
    let due_re = regex::Regex::new(r"(?:^|\s)due:\d{4}-\d{2}-\d{2}\b").unwrap();
    let stripped = due_re.replace_all(line, "").to_string();
    let stripped = stripped.trim_end();
    match due {
        Some(d) => format!("{} due:{}", stripped, d),
        None => stripped.to_string(),
    }
}

#[tauri::command]
pub async fn set_task_due(
    state: State<'_, AppState>,
    vault_path: String,
    note_path: String,
    line: usize,
    raw_line: String,
    due: Option<String>,
) -> Result<(), String> {
    // Validate format (YYYY-MM-DD); None/empty clears the due date.
    let date_re = regex::Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap();
    let due_val: Option<&str> = match due.as_deref() {
        None | Some("") => None,
        Some(d) if date_re.is_match(d) => Some(d),
        Some(_) => return Err("Invalid due date".to_string()),
    };

    let search = state.vault_snapshot(&vault_path)?;
    let _write_guard = state.note_save.lock().await;
    let NoteContent {
        meta,
        content: body,
        raw,
        ..
    } = operations::read_vault_note(&vault_path, &note_path)?;
    let mut lines: Vec<String> = body.lines().map(|l| l.to_string()).collect();
    let idx = if lines.get(line).map(|l| *l == raw_line).unwrap_or(false) {
        line
    } else {
        lines
            .iter()
            .position(|l| *l == raw_line)
            .ok_or("Task line not found (note changed)")?
    };

    let updated = set_due_on_line(&lines[idx], due_val);
    if updated == lines[idx] {
        return Ok(());
    }
    lines[idx] = updated;
    let mut new_body = lines.join("\n");
    if body.ends_with('\n') && !new_body.ends_with('\n') {
        new_body.push('\n');
    }
    if operations::save_note(&vault_path, &note_path, &meta, &new_body, &raw)?.is_some() {
        index_note_bg(search, &note_path);
    }
    Ok(())
}

// ── Search ──

#[tauri::command]
pub async fn search_notes(
    state: State<'_, AppState>,
    query: String,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<SearchPage, String> {
    let search = state
        .search_index
        .lock()
        .map_err(|error| error.to_string())?
        .clone()
        .ok_or("Search index not initialized")?;
    tauri::async_runtime::spawn_blocking(move || {
        search.search(&query, limit.unwrap_or(20), offset.unwrap_or(0))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn reindex(state: State<'_, AppState>) -> Result<(), String> {
    let (search, vault_path) = state.active_search()?;
    tauri::async_runtime::spawn_blocking(move || search.rebuild(&vault_path))
        .await
        .map_err(|error| error.to_string())?
}

// ── Trash ──

#[tauri::command]
pub async fn get_trash(state: State<'_, AppState>) -> Result<TrashContents, String> {
    let vault_path = state
        .config
        .lock()
        .map_err(|error| error.to_string())?
        .active_vault
        .clone()
        .ok_or("No active vault")?;
    tauri::async_runtime::spawn_blocking(move || operations::get_trash_contents(&vault_path))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn restore_note(
    state: State<'_, AppState>,
    vault_path: String,
    trash_path: String,
    dest_notebook: Option<String>,
) -> Result<String, String> {
    let search = state.vault_snapshot(&vault_path)?;
    let _write_guard = state.note_save.lock().await;
    let restored = operations::restore_note(&vault_path, &trash_path, dest_notebook.as_deref())?;
    index_note_bg(search, &restored);
    Ok(restored)
}

#[tauri::command]
pub async fn restore_notebook(
    state: State<'_, AppState>,
    vault_path: String,
    trash_path: String,
) -> Result<String, String> {
    let search = state.vault_snapshot(&vault_path)?;
    let _write_guard = state.note_save.lock().await;
    let restored = operations::restore_notebook(&vault_path, &trash_path)?;
    if let Some(search) = search {
        if let Err(error) = search.queue_vault_refresh(&vault_path) {
            log::error!("Failed to queue restored notebook search refresh: {error}");
        }
    }
    Ok(restored)
}

#[tauri::command]
pub async fn permanent_delete(
    state: State<'_, AppState>,
    vault_path: String,
    path: String,
) -> Result<(), String> {
    state.validate_vault(&vault_path)?;
    let _write_guard = state.note_save.lock().await;
    operations::permanent_delete(&vault_path, &path)
}

#[tauri::command]
pub async fn empty_trash(state: State<'_, AppState>, vault_path: String) -> Result<(), String> {
    state.validate_vault(&vault_path)?;
    let _write_guard = state.note_save.lock().await;
    operations::empty_trash(&vault_path)
}

// ── Vault State ──

#[tauri::command]
pub async fn load_vault_state(
    state: State<'_, AppState>,
    vault_path: String,
) -> Result<VaultState, String> {
    if state
        .config
        .lock()
        .map_err(|error| error.to_string())?
        .active_vault
        .as_deref()
        != Some(vault_path.as_str())
    {
        return Err("Vault changed before loading its state".to_string());
    }
    let read_guard = state.note_save.clone().lock_owned().await;
    tauri::async_runtime::spawn_blocking(move || {
        let _read_guard = read_guard;
        operations::load_vault_state(&vault_path)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn save_vault_state(
    state: State<'_, AppState>,
    vault_path: String,
    vault_state: VaultState,
) -> Result<(), String> {
    if state
        .config
        .lock()
        .map_err(|error| error.to_string())?
        .active_vault
        .as_deref()
        != Some(vault_path.as_str())
    {
        return Err("Vault changed before saving its state".to_string());
    }
    // Serialize state writes with the existing vault write queue so concurrent
    // navigation updates cannot finish out of order or race a state read.
    let save_guard = state.note_save.clone().lock_owned().await;
    tauri::async_runtime::spawn_blocking(move || {
        let _save_guard = save_guard;
        operations::save_vault_state(&vault_path, &vault_state)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod vault_state_scope_tests {
    use super::*;
    use futures::FutureExt;

    #[tokio::test]
    async fn queued_state_write_keeps_its_originating_vault() -> Result<(), String> {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        std::fs::create_dir(first.path().join(".helixnotes")).unwrap();
        std::fs::create_dir(second.path().join(".helixnotes")).unwrap();
        let first_path = first.path().to_string_lossy().into_owned();
        let second_path = second.path().to_string_lossy().into_owned();
        let first_state = VaultState {
            sidebar_width: 240.0,
            last_open_note: Some(first.path().join("A.md").to_string_lossy().into_owned()),
            ..VaultState::default()
        };
        let second_state = VaultState {
            sidebar_width: 360.0,
            ..VaultState::default()
        };
        operations::save_vault_state(&second_path, &second_state).unwrap();
        let app = tauri::test::mock_builder()
            .manage(AppState::new(AppConfig {
                active_vault: Some(first_path.clone()),
                ..AppConfig::default()
            }))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let state = app.state::<AppState>();
        let saving_note = state.note_save.lock().await;
        let mut pending = Box::pin(save_vault_state(
            app.state(),
            first_path.clone(),
            first_state.clone(),
        ));
        assert!(pending.as_mut().now_or_never().is_none());
        state
            .config
            .lock()
            .map_err(|error| error.to_string())?
            .active_vault = Some(second_path.clone());
        drop(saving_note);
        pending.await?;

        let saved_first = operations::load_vault_state(&first_path).unwrap();
        assert_eq!(saved_first.sidebar_width, 240.0);
        assert_eq!(saved_first.last_open_note, first_state.last_open_note);
        assert_eq!(
            operations::load_vault_state(&second_path)
                .unwrap()
                .sidebar_width,
            360.0
        );
        assert!(
            save_vault_state(app.state(), first_path.clone(), first_state)
                .await
                .is_err()
        );
        assert!(load_vault_state(app.state(), first_path).await.is_err());
        assert_eq!(
            load_vault_state(app.state(), second_path)
                .await?
                .sidebar_width,
            360.0
        );
        Ok(())
    }
}

// ── Clipboard ──

/// Copy text to the system clipboard through the native backend.
#[cfg(desktop)]
#[tauri::command]
pub fn copy_text_to_clipboard(text: String) -> Result<(), String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| format!("Clipboard init failed: {}", e))?;
    clipboard
        .set_text(text)
        .map_err(|e| format!("Failed to copy text: {}", e))
}

#[cfg(mobile)]
#[tauri::command]
pub fn copy_text_to_clipboard(_text: String) -> Result<(), String> {
    Err("Text clipboard copy is only supported on desktop".to_string())
}

/// Text and HTML on the system clipboard, read through the native backend.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct ClipboardContents {
    pub text: Option<String>,
    pub html: Option<String>,
}

/// Read text and HTML from the system clipboard through the native backend.
/// Works around the WebKitGTK clipboard bug where the async `navigator.clipboard`
/// API returns nothing, or only plain text, for content copied from external
/// applications. Each format is optional; a missing one is not an error.
#[cfg(desktop)]
#[tauri::command]
pub async fn read_clipboard_contents() -> Result<ClipboardContents, String> {
    // Reading the clipboard can block while the owning application answers, like the image read below.
    tauri::async_runtime::spawn_blocking(move || {
        let mut clipboard =
            arboard::Clipboard::new().map_err(|e| format!("Clipboard init failed: {}", e))?;
        let text = clipboard.get_text().ok().filter(|text| !text.is_empty());
        let html = clipboard.get().html().ok().filter(|html| !html.is_empty());
        Ok(ClipboardContents { text, html })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(mobile)]
#[tauri::command]
pub async fn read_clipboard_contents() -> Result<ClipboardContents, String> {
    Err("Native clipboard read is only supported on desktop".to_string())
}

/// Read image from system clipboard (bypasses WebKitGTK clipboard bug).
/// Returns PNG bytes as Vec<u8>, or error if no image on clipboard.
#[cfg(desktop)]
#[tauri::command]
pub async fn read_clipboard_image() -> Result<Vec<u8>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut clipboard =
            arboard::Clipboard::new().map_err(|e| format!("Clipboard init failed: {}", e))?;
        let img = clipboard
            .get_image()
            .map_err(|_| "No image on clipboard".to_string())?;
        // Encode RGBA data to PNG
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut encoder = png::Encoder::new(
                std::io::Cursor::new(&mut buf),
                img.width as u32,
                img.height as u32,
            );
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder
                .write_header()
                .map_err(|e| format!("PNG header failed: {}", e))?;
            writer
                .write_image_data(&img.bytes)
                .map_err(|e| format!("PNG encode failed: {}", e))?;
        }
        Ok(buf)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(mobile)]
#[tauri::command]
pub async fn read_clipboard_image() -> Result<Vec<u8>, String> {
    Err("Clipboard image reading not supported on Android".to_string())
}

/// Copy an image file to the system clipboard.
#[cfg(desktop)]
#[tauri::command]
pub async fn copy_image_to_clipboard(app: AppHandle, path: String) -> Result<(), String> {
    ensure_readable_path(&app, Path::new(&path))?;
    tauri::async_runtime::spawn_blocking(move || {
        let data = std::fs::read(&path).map_err(|e| format!("Failed to read image: {}", e))?;
        let img =
            image::load_from_memory(&data).map_err(|e| format!("Failed to decode image: {}", e))?;
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        let img_data = arboard::ImageData {
            width: w as usize,
            height: h as usize,
            bytes: std::borrow::Cow::Owned(rgba.into_raw()),
        };
        let mut clipboard =
            arboard::Clipboard::new().map_err(|e| format!("Clipboard init failed: {}", e))?;
        clipboard
            .set_image(img_data)
            .map_err(|e| format!("Failed to set clipboard image: {}", e))?;
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(mobile)]
#[tauri::command]
pub async fn copy_image_to_clipboard(_app: AppHandle, _path: String) -> Result<(), String> {
    Err("Clipboard image copy not supported on Android".to_string())
}

/// Copy PNG bytes directly to the system clipboard.
#[cfg(desktop)]
#[tauri::command]
pub async fn copy_png_to_clipboard(data: Vec<u8>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let img =
            image::load_from_memory(&data).map_err(|e| format!("Failed to decode image: {}", e))?;
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        let img_data = arboard::ImageData {
            width: w as usize,
            height: h as usize,
            bytes: std::borrow::Cow::Owned(rgba.into_raw()),
        };
        let mut clipboard =
            arboard::Clipboard::new().map_err(|e| format!("Clipboard init failed: {}", e))?;
        clipboard
            .set_image(img_data)
            .map_err(|e| format!("Failed to set clipboard image: {}", e))?;
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(mobile)]
#[tauri::command]
pub async fn copy_png_to_clipboard(_data: Vec<u8>) -> Result<(), String> {
    Err("Clipboard image copy not supported on Android".to_string())
}

// ── Attachments ──

#[tauri::command]
pub async fn save_image(
    state: State<'_, AppState>,
    vault_path: String,
    name: String,
    data: Vec<u8>,
) -> Result<String, String> {
    state.validate_vault(&vault_path)?;
    let write_guard = state.note_save.clone().lock_owned().await;
    tauri::async_runtime::spawn_blocking(move || {
        let _write_guard = write_guard;
        operations::save_image(&vault_path, &name, &data)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn save_attachment(
    state: State<'_, AppState>,
    vault_path: String,
    name: String,
    data: Vec<u8>,
) -> Result<String, String> {
    state.validate_vault(&vault_path)?;
    let write_guard = state.note_save.clone().lock_owned().await;
    tauri::async_runtime::spawn_blocking(move || {
        let _write_guard = write_guard;
        operations::save_attachment(&vault_path, &name, &data)
    })
    .await
    .map_err(|error| error.to_string())?
}

// ── Notebook Icons ──

#[tauri::command]
pub async fn get_notebook_icons(
    state: State<'_, AppState>,
    vault_path: String,
) -> Result<std::collections::HashMap<String, String>, String> {
    state.validate_vault(&vault_path)?;
    tauri::async_runtime::spawn_blocking(move || operations::load_notebook_icons(&vault_path))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn set_notebook_icon(
    state: State<'_, AppState>,
    vault_path: String,
    notebook_relative: String,
    icon_relative: Option<String>,
) -> Result<(), String> {
    state.validate_vault(&vault_path)?;
    let write_guard = state.note_save.clone().lock_owned().await;
    tauri::async_runtime::spawn_blocking(move || {
        let _write_guard = write_guard;
        operations::set_notebook_icon(&vault_path, &notebook_relative, icon_relative.as_deref())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn import_notebook_icon(
    state: State<'_, AppState>,
    vault_path: String,
    notebook_relative: String,
    name: String,
    data: Vec<u8>,
) -> Result<String, String> {
    state.validate_vault(&vault_path)?;
    let write_guard = state.note_save.clone().lock_owned().await;
    tauri::async_runtime::spawn_blocking(move || {
        let _write_guard = write_guard;
        operations::import_notebook_icon(&vault_path, &notebook_relative, &name, &data)
    })
    .await
    .map_err(|error| error.to_string())?
}

// ── Tag Styles ──

#[tauri::command]
pub async fn get_tag_styles(
    state: State<'_, AppState>,
    vault_path: String,
) -> Result<std::collections::HashMap<String, crate::types::TagStyle>, String> {
    state.validate_vault(&vault_path)?;
    tauri::async_runtime::spawn_blocking(move || operations::load_tag_styles(&vault_path))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn set_tag_style(
    state: State<'_, AppState>,
    vault_path: String,
    tag: String,
    change: TagStyleChange,
) -> Result<Option<TagStyle>, String> {
    state.validate_vault(&vault_path)?;
    let write_guard = state.note_save.clone().lock_owned().await;
    tauri::async_runtime::spawn_blocking(move || {
        let _write_guard = write_guard;
        operations::patch_tag_style(&vault_path, &tag, change)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn import_tag_icon(
    state: State<'_, AppState>,
    vault_path: String,
    tag: String,
    name: String,
    data: Vec<u8>,
) -> Result<TagStyle, String> {
    state.validate_vault(&vault_path)?;
    let write_guard = state.note_save.clone().lock_owned().await;
    tauri::async_runtime::spawn_blocking(move || {
        let _write_guard = write_guard;
        operations::import_tag_icon(&vault_path, &tag, &name, &data)
    })
    .await
    .map_err(|error| error.to_string())?
}

// ── General Settings ──

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn set_general_settings(
    state: State<'_, AppState>,
    compact_notes: bool,
    time_format: String,
    week_start: String,
    daily_title_format: String,
    gpu_acceleration: bool,
    autostart: bool,
    pdf_preview: bool,
    pdf_height: u32,
    title_mode: String,
    hide_title_in_body: bool,
    show_line_numbers: bool,
    show_link_arrows: bool,
    default_view_mode: bool,
    new_notes_in_source_mode: bool,
    show_tray_icon: bool,
    close_to_tray: bool,
    enable_wiki_links: bool,
    show_note_dates: bool,
    show_note_switcher: bool,
    startup_view: StartupView,
    restore_last_session: bool,
    show_all_notes: bool,
    show_quick_access: bool,
    show_tasks: bool,
    show_daily_notes: bool,
    show_trash: bool,
) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    config.compact_notes = compact_notes;
    config.show_note_dates = show_note_dates;
    config.show_note_switcher = show_note_switcher;
    config.startup_view = startup_view;
    config.restore_last_session = restore_last_session;
    config.show_all_notes = show_all_notes;
    config.show_quick_access = show_quick_access;
    config.show_tasks = show_tasks;
    config.show_daily_notes = show_daily_notes;
    config.show_trash = show_trash;
    config.time_format = time_format;
    config.week_start = week_start;
    config.daily_title_format = daily_title_format;
    config.gpu_acceleration = gpu_acceleration;
    config.autostart = autostart;
    config.pdf_preview = pdf_preview;
    config.pdf_height = pdf_height;
    config.title_mode = title_mode;
    config.hide_title_in_body = hide_title_in_body;
    config.show_line_numbers = show_line_numbers;
    config.show_link_arrows = show_link_arrows;
    config.default_view_mode = default_view_mode;
    config.new_notes_in_source_mode = new_notes_in_source_mode;
    config.show_tray_icon = show_tray_icon;
    config.close_to_tray = close_to_tray;
    config.enable_wiki_links = enable_wiki_links;
    save_app_config(&config)?;
    Ok(())
}

// ── Quick Access ──

#[tauri::command]
pub async fn get_quick_access(
    state: State<'_, AppState>,
    vault_path: String,
) -> Result<Vec<NoteEntry>, String> {
    state.validate_vault(&vault_path)?;
    let read_guard = state.note_save.clone().lock_owned().await;
    tauri::async_runtime::spawn_blocking(move || {
        let _read_guard = read_guard;
        operations::get_quick_access_notes(&vault_path)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn add_quick_access(
    state: State<'_, AppState>,
    vault_path: String,
    note_relative: String,
) -> Result<(), String> {
    state.validate_vault(&vault_path)?;
    let _write_guard = state.note_save.lock().await;
    operations::add_quick_access(&vault_path, &note_relative)
}

#[tauri::command]
pub async fn remove_quick_access(
    state: State<'_, AppState>,
    vault_path: String,
    note_relative: String,
) -> Result<(), String> {
    state.validate_vault(&vault_path)?;
    let _write_guard = state.note_save.lock().await;
    operations::remove_quick_access(&vault_path, &note_relative)
}

#[tauri::command]
pub async fn reorder_quick_access(
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<(), String> {
    let vault_path = state
        .config
        .lock()
        .map_err(|error| error.to_string())?
        .active_vault
        .clone()
        .ok_or("No active vault")?;
    let _write_guard = state.note_save.lock().await;
    operations::save_quick_access(&vault_path, &paths)
}

#[tauri::command]
pub async fn get_vault_stats(state: State<'_, AppState>) -> Result<VaultStats, String> {
    let vault_path = state
        .config
        .lock()
        .map_err(|error| error.to_string())?
        .active_vault
        .clone()
        .ok_or("No active vault")?;
    tauri::async_runtime::spawn_blocking(move || scan_vault_stats(&vault_path))
        .await
        .map_err(|error| error.to_string())?
}

fn scan_vault_stats(vault_path: &str) -> Result<VaultStats, String> {
    use rayon::prelude::*;
    let vault = Path::new(vault_path);

    let paths: Vec<PathBuf> = walkdir::WalkDir::new(vault)
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            let Ok(relative) = entry.path().strip_prefix(vault) else {
                return false;
            };
            let is_directory = entry.file_type().is_dir();
            if is_directory && entry.file_name() == ".trash" {
                return false;
            }
            // Prune metadata children except the attachments directory. Inspect
            // only vault-relative components, never names in the vault's parent.
            if relative
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|name| name == ".helixnotes")
            {
                return is_directory && entry.file_name() == "attachments";
            }
            true
        })
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .collect();

    // Stat files in parallel; each metadata() is a FUSE round-trip on Android.
    let (total_notes, total_attachments, notes_size, attachments_size) = paths
        .par_iter()
        .map(|path| {
            let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                (1u64, 0u64, size, 0u64)
            } else {
                (0u64, 1u64, 0u64, size)
            }
        })
        .reduce(
            || (0u64, 0u64, 0u64, 0u64),
            |a, b| (a.0 + b.0, a.1 + b.1, a.2 + b.2, a.3 + b.3),
        );

    Ok(VaultStats {
        total_notes,
        total_attachments,
        notes_size,
        attachments_size,
        total_size: notes_size + attachments_size,
    })
}

#[cfg(test)]
mod vault_stats_tests {
    use super::scan_vault_stats;
    use std::path::Path;

    fn write_fixture(vault: &Path, files: &[(&[&str], &[u8])]) {
        for (components, contents) in files {
            let mut path = vault.to_path_buf();
            for component in *components {
                path.push(component);
            }
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }
    }

    fn assert_stats(vault: &Path, expected: (u64, u64, u64, u64, u64)) {
        let stats = scan_vault_stats(vault.to_str().unwrap()).unwrap();
        assert_eq!(
            (
                stats.total_notes,
                stats.total_attachments,
                stats.notes_size,
                stats.attachments_size,
                stats.total_size,
            ),
            expected,
            "vault: {}",
            vault.display()
        );
    }

    #[test]
    fn vault_stats_preserve_file_classification_and_byte_totals() {
        let vault = tempfile::tempdir().unwrap();
        write_fixture(
            vault.path(),
            &[
                (&["Note.md"], b"note"),
                (&["Notebook", "Note.md"], "café".as_bytes()),
                (&["Empty.md"], b""),
                (&["Upper.MD"], b"upper"),
                (&["Notebook", "data.bin"], b"\0\x01\xff"),
                (&["README"], b"plain"),
            ],
        );

        assert_stats(vault.path(), (3, 3, 9, 13, 22));
    }

    #[test]
    fn vault_stats_include_only_attachment_subtrees_inside_metadata() {
        let vault = tempfile::tempdir().unwrap();
        write_fixture(
            vault.path(),
            &[
                (&[".helixnotes", "attachments", "image.png"], b"123"),
                (&[".helixnotes", "attachments", "Manual.md"], b"1234"),
                (
                    &[".helixnotes", "attachments", "Nested", "icon.svg"],
                    b"12345",
                ),
                (
                    &[".helixnotes", "attachments", ".trash-backup", "backup.bin"],
                    b"123456",
                ),
                (&[".helixnotes", "settings.json"], b"excluded"),
                (&[".helixnotes", "history", "Snapshot.md"], b"excluded"),
                (&[".helixnotes", "trash", "Deleted.md"], b"excluded"),
                (&[".helixnotes", "attachments-old", "old.png"], b"excluded"),
                (
                    &[".helixnotes", "attachments", ".trash", "deleted.png"],
                    b"excluded",
                ),
            ],
        );

        assert_stats(vault.path(), (1, 3, 4, 14, 18));
    }

    #[test]
    fn vault_stats_match_exact_directory_boundaries() {
        let vault = tempfile::tempdir().unwrap();
        write_fixture(
            vault.path(),
            &[
                (&[".trash-backup", "Note.md"], b"1"),
                (&[".helixnotes-backup", "Note.md"], b"12"),
                (&["attachments-old", "Note.md"], b"123"),
                (&["Work.notebook", "attachments-old", "Note.md"], b"1234"),
                (&["Work.notebook", ".trash-backup", "picture.png"], b"12345"),
                (&["Files", ".trash"], b"123456"),
                (&["Files", ".helixnotes"], b"1234567"),
                (
                    &["Work.notebook", ".helixnotes", "attachments", "photo.png"],
                    b"12345678",
                ),
                (&[".trash", "Deleted.md"], b"excluded"),
                (&["Work.notebook", ".trash", "deleted.png"], b"excluded"),
                (
                    &["Work.notebook", ".helixnotes", "history", "Snapshot.md"],
                    b"excluded",
                ),
                (
                    &["Work.notebook", ".helixnotes", "attachments-old", "old.png"],
                    b"excluded",
                ),
            ],
        );

        assert_stats(vault.path(), (4, 4, 10, 26, 36));
    }

    #[test]
    fn vault_stats_ignore_excluded_names_outside_the_vault() {
        let locations: &[&[&str]] = &[
            &[".trash", "vault"],
            &[".helixnotes", "history", "vault"],
            &[".helixnotes", "attachments-old", "vault"],
            &[".trash"],
            &[".helixnotes"],
        ];
        for components in locations {
            let root = tempfile::tempdir().unwrap();
            let mut vault = root.path().to_path_buf();
            for component in *components {
                vault.push(component);
            }
            write_fixture(
                &vault,
                &[
                    (&["Note.md"], b"hello"),
                    (&["Notebook", "photo.png"], b"123"),
                    (&[".helixnotes", "attachments", "photo.png"], b"12"),
                    (&[".helixnotes", "history", "Snapshot.md"], b"excluded"),
                    (&[".trash", "Deleted.md"], b"excluded"),
                ],
            );

            assert_stats(&vault, (1, 2, 5, 5, 10));
        }
    }
}

#[tauri::command]
pub fn import_obsidian(app: AppHandle) -> Result<(), String> {
    let vault_path = {
        let state = app.state::<AppState>();
        let config = state.config.lock().map_err(|e| e.to_string())?;
        config
            .active_vault
            .clone()
            .ok_or("No active vault".to_string())?
    };
    // Fire-and-forget: return immediately, do work in background thread
    std::thread::spawn(move || {
        use tauri::Emitter;
        match do_import_obsidian(app.clone(), &vault_path) {
            Ok(result) => {
                let _ = app.emit(
                    "import-done",
                    serde_json::json!({
                        "success": true,
                        "files_converted": result.files_converted,
                        "links_converted": result.links_converted,
                        "frontmatter_normalized": result.frontmatter_normalized,
                        "syntax_converted": result.syntax_converted,
                        "attachments_moved": result.attachments_moved,
                    }),
                );
            }
            Err(e) => {
                let _ = app.emit(
                    "import-done",
                    serde_json::json!({
                        "success": false,
                        "error": e,
                    }),
                );
            }
        }
    });
    Ok(())
}

fn do_import_obsidian(app: AppHandle, vault_path: &str) -> Result<ImportResult, String> {
    let state = app.state::<AppState>();
    let _maintenance_guard = state.vault_maintenance.blocking_lock();
    let _write_guard = state.note_save.blocking_lock();
    ensure_cleanup_idle(&state)?;
    state
        .importing
        .store(true, std::sync::atomic::Ordering::Relaxed);

    let result = crate::vault::import::import(vault_path);

    state
        .importing
        .store(false, std::sync::atomic::Ordering::Relaxed);

    result
}

// ── Orphaned attachment cleanup ──

#[derive(serde::Serialize)]
pub struct OrphanAttachment {
    pub name: String,
    pub size: u64,
}

// Conservatively find files in .helixnotes/attachments not referenced by ANY note. Scans every
// .md in the vault (including .helixnotes/trash, so a restorable trashed note keeps its files)
// plus notebook_icons.json and tag_styles.json. A single matcher finds all basename substrings
// in each source, first raw and then decoded in place (including URL-encoded paths).
// When in doubt a file is KEPT: a leftover orphan is harmless, a wrong deletion is not.
fn scan_orphaned_attachments(vault: &str) -> Result<Vec<(String, u64)>, String> {
    let attachments_dir = operations::helixnotes_dir(vault).join("attachments");
    let entries = match std::fs::read_dir(&attachments_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let mut files: Vec<(String, u64)> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path();
        let metadata = p.metadata().map_err(|error| error.to_string())?;
        if metadata.is_file() {
            let name = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let size = metadata.len();
            files.push((name, size));
        }
    }
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let matcher = aho_corasick::AhoCorasick::new(files.iter().map(|(name, _)| name.as_str()))
        .map_err(|error| error.to_string())?;
    let mut referenced = vec![false; files.len()];
    let mut scan_source = |content: String| {
        // Overlapping matches retain shorter names contained in another attachment's name.
        for found in matcher.find_overlapping_iter(content.as_bytes()) {
            referenced[found.pattern().as_usize()] = true;
        }
        if !content.contains('%') {
            return;
        }
        let decoded = percent_decode(content);
        for found in matcher.find_overlapping_iter(&decoded) {
            referenced[found.pattern().as_usize()] = true;
        }
    };
    for entry in walkdir::WalkDir::new(vault) {
        let entry = entry.map_err(|error| error.to_string())?;
        let p = entry.path();
        if p.extension().and_then(|x| x.to_str()) == Some("md")
            && p.metadata().map_err(|error| error.to_string())?.is_file()
        {
            let content = std::fs::read_to_string(p).map_err(|error| error.to_string())?;
            scan_source(content);
        }
    }
    // Folder and tag icons live in attachments but are referenced here, not in notes. (#157)
    for mapping in ["notebook_icons.json", "tag_styles.json"] {
        let mapping_path = operations::helixnotes_dir(vault).join(mapping);
        match std::fs::read_to_string(&mapping_path) {
            Ok(content) => {
                let references = operations::attachment_metadata_references(mapping, &content)
                    .map_err(|error| format!("Invalid {mapping}: {error}"))?;
                scan_source(content);
                // JSON escapes (including Unicode) are references too. Retain
                // both the raw representation and decoded string values.
                for reference in references {
                    scan_source(reference);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    let orphans = files
        .into_iter()
        .enumerate()
        .filter_map(|(id, file)| (!referenced[id]).then_some(file))
        .collect();
    Ok(orphans)
}

#[tauri::command]
pub async fn find_orphaned_attachments(
    state: State<'_, AppState>,
    vault_path: String,
) -> Result<Vec<OrphanAttachment>, String> {
    state.validate_vault(&vault_path)?;
    let save_guard = state.note_save.clone().lock_owned().await;
    ensure_cleanup_idle(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _save_guard = save_guard;
        let orphans = scan_orphaned_attachments(&vault_path)?;
        Ok(orphans
            .into_iter()
            .map(|(name, size)| OrphanAttachment { name, size })
            .collect())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn trash_orphaned_attachments(
    state: State<'_, AppState>,
    vault_path: String,
    names: Vec<String>,
) -> Result<u32, String> {
    state.validate_vault(&vault_path)?;
    let save_guard = state.note_save.clone().lock_owned().await;
    ensure_cleanup_idle(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _save_guard = save_guard;
        trash_orphaned_files(&vault_path, &names)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn ensure_cleanup_idle(state: &AppState) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    if state.importing.load(Ordering::Acquire) || state.syncing.load(Ordering::Acquire) {
        Err("Attachment cleanup is unavailable while import or sync is running".to_string())
    } else {
        Ok(())
    }
}

fn trash_orphaned_files(vault: &str, names: &[String]) -> Result<u32, String> {
    // Re-scan now and only move files that are STILL orphaned (guards against a reference added
    // between scan and confirm), intersected with the caller's selection.
    let current: std::collections::HashSet<String> = scan_orphaned_attachments(vault)?
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    let attachments_dir = operations::helixnotes_dir(vault).join("attachments");
    let trash_dir = operations::helixnotes_dir(vault).join("trash");
    std::fs::create_dir_all(&trash_dir).map_err(|e| e.to_string())?;
    let stamp = chrono::Utc::now().format("%Y%m%d%H%M%S%3f").to_string();
    let mut moved = 0u32;
    for (i, name) in names.iter().enumerate() {
        if !current.contains(name) {
            continue;
        }
        // Path-traversal guard: only act on a bare filename inside the attachments dir.
        if std::path::Path::new(name)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .as_deref()
            != Some(name.as_str())
        {
            continue;
        }
        let src = attachments_dir.join(name);
        match src.metadata() {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.to_string()),
        }
        let dest = trash_dir.join(format!("{}_{}_attachment_{}", stamp, i, name));
        std::fs::rename(&src, &dest).map_err(|error| error.to_string())?;
        moved += 1;
    }
    Ok(moved)
}

#[cfg(test)]
mod orphan_attachment_tests {
    use super::{scan_orphaned_attachments, trash_orphaned_files};
    use crate::vault::operations;

    #[test]
    fn invalid_icon_metadata_aborts_scan_and_removal_without_losing_attachments() {
        for (mapping, content) in [
            ("notebook_icons.json", "{"),
            ("notebook_icons.json", r#"{"Notebook":42}"#),
            ("tag_styles.json", r#"{"Work":"not a style"}"#),
            ("tag_styles.json", r#"{"Work":{"icon":false}}"#),
            ("tag_styles.json", r#"{"Work":{"color":"red"}}"#),
            ("tag_styles.json", r#"{"Work":{"unexpected":"keep.png"}}"#),
        ] {
            let root = tempfile::tempdir().unwrap();
            let vault = root.path().to_str().unwrap();
            operations::ensure_vault_structure(vault).unwrap();
            let storage = operations::helixnotes_dir(vault);
            let attachment = storage.join("attachments/keep.png");
            std::fs::write(&attachment, b"preserved").unwrap();
            std::fs::write(storage.join(mapping), content).unwrap();
            assert!(
                scan_orphaned_attachments(vault).is_err(),
                "{mapping}: {content}"
            );
            assert!(trash_orphaned_files(vault, &["keep.png".into()]).is_err());
            assert_eq!(std::fs::read(&attachment).unwrap(), b"preserved");
        }
    }

    #[test]
    fn json_escaped_unicode_icon_references_survive_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let vault = root.path().to_str().unwrap();
        operations::ensure_vault_structure(vault).unwrap();
        let storage = operations::helixnotes_dir(vault);
        std::fs::write(storage.join("attachments/猫.png"), b"notebook").unwrap();
        std::fs::write(storage.join("attachments/é.png"), b"tag").unwrap();
        std::fs::write(
            storage.join("notebook_icons.json"),
            r#"{"Book":"attachments/\u732b.png"}"#,
        )
        .unwrap();
        std::fs::write(
            storage.join("tag_styles.json"),
            r#"{"Work":{"icon":"attachments/\u00e9.png"}}"#,
        )
        .unwrap();
        assert_eq!(
            trash_orphaned_files(vault, &["猫.png".into(), "é.png".into()]).unwrap(),
            0
        );
        assert_eq!(
            std::fs::read(storage.join("attachments/猫.png")).unwrap(),
            b"notebook"
        );
        assert_eq!(
            std::fs::read(storage.join("attachments/é.png")).unwrap(),
            b"tag"
        );
    }

    #[test]
    fn cleanup_keeps_plain_overlapping_and_literal_percent_basenames() {
        let root = tempfile::tempdir().unwrap();
        let vault = root.path().to_str().unwrap();
        operations::ensure_vault_structure(vault).unwrap();
        let attachments = operations::helixnotes_dir(vault).join("attachments");
        for name in [
            "plain.txt",
            "icon.png",
            "folder-icon.png",
            "folder-icon.png.backup",
            "literal%20space.png",
            "literal space.png",
            "orphan.png",
        ] {
            std::fs::write(attachments.join(name), b"image").unwrap();
        }
        std::fs::write(
            root.path().join("Mentions.md"),
            "Not links: prefixplain.txtsuffix folder-icon.png.backup literal%20space.png",
        )
        .unwrap();

        assert_eq!(
            scan_orphaned_attachments(vault).unwrap(),
            vec![("orphan.png".into(), 5)]
        );
    }

    #[test]
    fn cleanup_keeps_unicode_and_encoded_references_from_every_source() {
        let root = tempfile::tempdir().unwrap();
        let vault = root.path().to_str().unwrap();
        operations::ensure_vault_structure(vault).unwrap();
        let storage = operations::helixnotes_dir(vault);
        let attachments = storage.join("attachments");
        for name in [
            "raw 猫.png",
            "猫 spaced.png",
            "café.png",
            "%Q猫 image.png",
            "%G1broken image.png",
            "after-bad image.png",
            "trailing%",
            "trash ü.png",
            "history ね.png",
            "📁 icon.png",
            "é tag.png",
            "映像.png",
            "orphan.png",
        ] {
            std::fs::write(attachments.join(name), b"image").unwrap();
        }
        std::fs::write(
            root.path().join("Unicode.md"),
            "raw 猫.png 猫%20spaced.png %63%61%66%c3%a9.png\n\
             %Q猫%20image.png %G1broken%20image.png %FFafter-bad%20image.png trailing%",
        )
        .unwrap();
        std::fs::write(storage.join("trash/old.md"), "![trash](trash%20%C3%BC.png)").unwrap();
        std::fs::create_dir_all(storage.join("history/Old")).unwrap();
        std::fs::write(
            storage.join("history/Old/1.md"),
            "![history](history%20%E3%81%AD.png)",
        )
        .unwrap();
        std::fs::write(
            storage.join("notebook_icons.json"),
            r#"{"Notebook":"attachments/%F0%9F%93%81%20icon.png"}"#,
        )
        .unwrap();
        std::fs::write(
            storage.join("tag_styles.json"),
            r#"{"encoded":{"icon":"attachments/%C3%A9%20tag.png"},"raw":{"icon":"attachments/映像.png"}}"#,
        )
        .unwrap();

        assert_eq!(
            scan_orphaned_attachments(vault).unwrap(),
            vec![("orphan.png".into(), 5)]
        );
    }

    #[test]
    fn unreadable_mapping_aborts_even_after_all_attachments_are_referenced() {
        let root = tempfile::tempdir().unwrap();
        let vault = root.path().to_str().unwrap();
        operations::ensure_vault_structure(vault).unwrap();
        let storage = operations::helixnotes_dir(vault);
        let attachment = storage.join("attachments/keep.png");
        std::fs::write(&attachment, b"image").unwrap();
        std::fs::write(root.path().join("Note.md"), "keep.png").unwrap();
        std::fs::write(storage.join("notebook_icons.json"), [0xff]).unwrap();

        assert!(trash_orphaned_files(vault, &["keep.png".into()]).is_err());
        assert_eq!(std::fs::read(attachment).unwrap(), b"image");
    }

    #[test]
    fn cleanup_revalidates_references_and_only_trashes_selected_orphans() {
        let root = tempfile::tempdir().unwrap();
        let vault = root.path().to_str().unwrap();
        operations::ensure_vault_structure(vault).unwrap();
        let storage = operations::helixnotes_dir(vault);
        let attachments = storage.join("attachments");
        for name in [
            "late.png",
            "trashed image.png",
            "icon.png",
            "orphan.png",
            "unselected.png",
        ] {
            std::fs::write(attachments.join(name), b"image").unwrap();
        }
        std::fs::write(
            storage.join("trash/old.md"),
            "![kept](../attachments/trashed%20image.png)",
        )
        .unwrap();
        std::fs::write(
            storage.join("notebook_icons.json"),
            r#"{"Notebook":"attachments/icon.png"}"#,
        )
        .unwrap();
        let mut found: Vec<_> = scan_orphaned_attachments(vault)
            .unwrap()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        found.sort();
        assert_eq!(found, ["late.png", "orphan.png", "unselected.png"]);

        std::fs::write(root.path().join("New.md"), "![added](late.png)").unwrap();
        let selected = vec!["late.png".into(), "orphan.png".into(), "../New.md".into()];
        assert_eq!(trash_orphaned_files(vault, &selected).unwrap(), 1);
        for name in [
            "late.png",
            "trashed image.png",
            "icon.png",
            "unselected.png",
        ] {
            assert_eq!(std::fs::read(attachments.join(name)).unwrap(), b"image");
        }
        assert!(!attachments.join("orphan.png").exists());
        assert_eq!(
            std::fs::read_to_string(root.path().join("New.md")).unwrap(),
            "![added](late.png)"
        );
        let trashed = std::fs::read_dir(storage.join("trash"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .ends_with("_attachment_orphan.png")
            })
            .unwrap();
        assert_eq!(std::fs::read(trashed).unwrap(), b"image");
    }

    #[test]
    fn unreadable_reference_sources_abort_cleanup_without_moving_files() {
        let root = tempfile::tempdir().unwrap();
        let vault = root.path().to_str().unwrap();
        operations::ensure_vault_structure(vault).unwrap();
        let storage = operations::helixnotes_dir(vault);
        let attachment = storage.join("attachments/keep.png");
        std::fs::write(&attachment, b"image").unwrap();
        let note = root.path().join("Unreadable.md");
        std::fs::write(&note, [0xff, 0xfe]).unwrap();
        assert!(trash_orphaned_files(vault, &["keep.png".into()]).is_err());
        assert_eq!(std::fs::read(&attachment).unwrap(), b"image");

        std::fs::remove_file(note).unwrap();
        std::fs::create_dir(storage.join("tag_styles.json")).unwrap();
        assert!(trash_orphaned_files(vault, &["keep.png".into()]).is_err());
        assert_eq!(std::fs::read(attachment).unwrap(), b"image");
    }
}

fn percent_decode(s: String) -> Vec<u8> {
    // Reuse the source buffer and match bytes, so Unicode is neither re-encoded nor made lossy.
    // Invalid/truncated escapes remain literal; only a complete %HH consumes three bytes.
    let mut bytes = s.into_bytes();
    let mut read = 0;
    let mut write = 0;
    while read < bytes.len() {
        if bytes[read] == b'%' && read + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (
                (bytes[read + 1] as char).to_digit(16),
                (bytes[read + 2] as char).to_digit(16),
            ) {
                bytes[write] = ((hi << 4) | lo) as u8;
                read += 3;
                write += 1;
                continue;
            }
        }
        bytes[write] = bytes[read];
        read += 1;
        write += 1;
    }
    bytes.truncate(write);
    bytes
}

// ── Open files and URLs with the system default handler ──

fn active_vault_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let state = app.state::<AppState>();
    let config = state.config.lock().map_err(|error| error.to_string())?;
    config
        .active_vault
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| "No active vault".to_string())
}

fn path_is_in_active_vault<R: Runtime>(app: &AppHandle<R>, path: &Path) -> bool {
    let Ok(vault) = active_vault_path(app)
        .and_then(|path| std::fs::canonicalize(path).map_err(|error| error.to_string()))
    else {
        return false;
    };
    let canonical = if path.exists() {
        std::fs::canonicalize(path).ok()
    } else {
        path.parent()
            .and_then(|parent| std::fs::canonicalize(parent).ok())
    };
    canonical.is_some_and(|path| path.starts_with(vault))
}

fn ensure_scoped_path<R: Runtime>(app: &AppHandle<R>, path: &Path) -> Result<(), String> {
    if app.fs_scope().is_allowed(path) {
        Ok(())
    } else {
        Err("Path was not selected by the user".to_string())
    }
}

fn ensure_readable_path<R: Runtime>(app: &AppHandle<R>, path: &Path) -> Result<(), String> {
    if path.is_file() && (path_is_in_active_vault(app, path) || app.fs_scope().is_allowed(path)) {
        Ok(())
    } else {
        Err("File must be inside the active vault or selected by the user".to_string())
    }
}

fn ensure_writable_path<R: Runtime>(app: &AppHandle<R>, path: &Path) -> Result<(), String> {
    if path_is_in_active_vault(app, path) || app.fs_scope().is_allowed(path) {
        Ok(())
    } else {
        Err("Destination must be inside the active vault or selected by the user".to_string())
    }
}

fn validate_external_url(url: &str) -> Result<(), String> {
    if url.chars().any(char::is_whitespace) {
        return Err("URL must not contain whitespace".to_string());
    }
    let parsed = reqwest::Url::parse(url).map_err(|_| "Invalid URL".to_string())?;
    match parsed.scheme() {
        "http" | "https" | "mailto" | "tel" | "sms" => Ok(()),
        _ => Err("Unsupported URL scheme".to_string()),
    }
}

#[cfg(target_os = "linux")]
fn open_linux(argument: &std::ffi::OsStr) -> Result<(), String> {
    let mut command = std::process::Command::new("xdg-open");
    command.arg(argument);
    if std::env::var("APPIMAGE").is_ok() {
        command
            .env_remove("LD_LIBRARY_PATH")
            .env_remove("LD_PRELOAD")
            .env_remove("GIO_LAUNCHED_DESKTOP_FILE")
            .env_remove("GIO_LAUNCHED_DESKTOP_FILE_PID");
        if let Ok(original_path) = std::env::var("PATH_ORIG") {
            command.env("PATH", original_path);
        }
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Failed to open item: {error}"))
}

fn open_path_with_system(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    return open_linux(path.as_os_str());

    #[cfg(not(target_os = "linux"))]
    tauri_plugin_opener::open_path(path, None::<&str>).map_err(|error| error.to_string())
}

fn open_url_with_system(url: &str) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    return open_linux(std::ffi::OsStr::new(url));

    #[cfg(not(target_os = "linux"))]
    tauri_plugin_opener::open_url(url, None::<&str>).map_err(|error| error.to_string())
}

#[cfg(test)]
mod external_access_tests {
    use super::{
        ensure_readable_path, ensure_writable_path, read_note, save_note, validate_external_url,
    };
    use crate::state::AppState;
    use crate::types::AppConfig;
    use std::fs;
    use tauri::Manager;
    use tauri_plugin_fs::FsExt;

    #[test]
    fn external_urls_allow_supported_schemes() {
        for url in [
            "https://helixnotes.com",
            "http://example.com",
            "mailto:hello@example.com",
            "tel:+123456789",
            "sms:+123456789",
        ] {
            assert!(validate_external_url(url).is_ok(), "{url}");
        }
    }

    #[test]
    fn external_urls_reject_commands_and_unsupported_schemes() {
        for url in [
            "https://example.com & calc.exe",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "not a URL",
        ] {
            assert!(validate_external_url(url).is_err(), "{url}");
        }
    }

    #[tokio::test]
    async fn selected_external_markdown_can_be_viewed_but_not_saved_in_place() {
        let root = tempfile::tempdir().unwrap();
        let vault = root.path().join("vault");
        let vault_path = vault.to_string_lossy().into_owned();
        crate::vault::operations::ensure_vault_structure(&vault_path).unwrap();
        let outside = root.path().join("café 雪.md");
        let raw = "---\ntitle: External note\ncustom: preserved\n---\n# External note\n\n![Image](images/picture.png)\n";
        fs::write(&outside, raw).unwrap();
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_fs::init())
            .manage(AppState::new(AppConfig {
                active_vault: Some(vault_path.clone()),
                ..Default::default()
            }))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let path = outside.to_string_lossy().into_owned();

        assert!(read_note(
            app.handle().clone(),
            app.state(),
            vault_path.clone(),
            path.clone()
        )
        .await
        .is_err());
        app.fs_scope().allow_file(&outside).unwrap();
        let note = read_note(
            app.handle().clone(),
            app.state(),
            vault_path.clone(),
            path.clone(),
        )
        .await
        .expect("an explicitly selected external Markdown file must open in the viewer");
        assert_eq!(note.path, path);
        assert_eq!(note.raw, raw);
        assert_eq!(note.meta.title, "External note");
        assert!(note.content.contains("![Image](images/picture.png)"));
        assert!(save_note(
            app.state(),
            vault_path,
            path,
            note.meta,
            "Changed".into(),
            note.raw
        )
        .await
        .is_err());
        assert_eq!(fs::read_to_string(&outside).unwrap(), raw);
    }

    #[tokio::test]
    async fn external_import_requires_scoped_attachments_and_keeps_its_queued_vault() {
        use futures::FutureExt;
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("A").to_string_lossy().into_owned();
        let b = root.path().join("B").to_string_lossy().into_owned();
        crate::vault::operations::ensure_vault_structure(&a).unwrap();
        crate::vault::operations::ensure_vault_structure(&b).unwrap();
        let external = root.path().join("external");
        fs::create_dir(&external).unwrap();
        let source = external.join("External.md");
        let image = external.join("photo.png");
        let raw = "![Image](photo.png)";
        fs::write(&source, raw).unwrap();
        fs::write(&image, b"image bytes").unwrap();
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_fs::init())
            .manage(AppState::new(AppConfig {
                active_vault: Some(a.clone()),
                vaults: vec![
                    crate::types::VaultConfig {
                        path: a.clone(),
                        ..Default::default()
                    },
                    crate::types::VaultConfig {
                        path: b.clone(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        app.fs_scope().allow_file(&source).unwrap();
        let plan = || {
            vec![crate::vault::operations::ExternalNoteReference {
                from: raw.find("photo.png").unwrap(),
                to: raw.find("photo.png").unwrap() + "photo.png".len(),
                href: "photo.png".into(),
                html: false,
                pdf: false,
            }]
        };
        let run = || {
            super::import_note(
                app.handle().clone(),
                app.state(),
                a.clone(),
                source.to_string_lossy().into_owned(),
                None,
                raw.into(),
                plan(),
            )
        };
        assert!(run().await.unwrap_err().contains("selected"));
        assert!(!std::path::Path::new(&a).join("External.md").exists());
        app.fs_scope().allow_file(&image).unwrap();
        let state = app.state::<AppState>();
        let writes = state.note_save.lock().await;
        let mut importing = Box::pin(run());
        assert!(importing.as_mut().now_or_never().is_none());
        state.config.lock().unwrap().active_vault = Some(b.clone());
        drop(writes);
        let note = importing.await.unwrap();
        assert!(std::path::Path::new(&note.path).starts_with(&a));
        assert!(!std::path::Path::new(&b).join("External.md").exists());
        assert_eq!(fs::read_to_string(source).unwrap(), raw);
        assert_eq!(fs::read(image).unwrap(), b"image bytes");
    }

    #[tokio::test]
    async fn imported_external_managed_attachments_use_the_source_vault_with_real_scopes() {
        use crate::{asset_scope, types::VaultConfig, vault::operations};
        let root = tempfile::tempdir().unwrap();
        let source_root = root.path().join("source");
        let destination = root
            .path()
            .join("destination")
            .to_string_lossy()
            .into_owned();
        operations::ensure_vault_structure(source_root.to_str().unwrap()).unwrap();
        operations::ensure_vault_structure(&destination).unwrap();
        std::fs::create_dir(source_root.join("Notebook")).unwrap();
        let source = source_root.join("Notebook/Note.md");
        let image = source_root.join(".helixnotes/attachments/photo%20).png");
        let href = ".helixnotes/attachments/photo%2520%29.png";
        let raw = format!("![Image]({href})");
        std::fs::write(&source, &raw).unwrap();
        std::fs::write(&image, b"source image").unwrap();
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_fs::init())
            .manage(AppState::new(AppConfig {
                active_vault: Some(destination.clone()),
                vaults: vec![VaultConfig {
                    path: source_root.to_string_lossy().into_owned(),
                    ..Default::default()
                }],
                ..Default::default()
            }))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        app.fs_scope().allow_file(&source).unwrap();
        app.fs_scope()
            .allow_directory(source.parent().unwrap(), true)
            .unwrap();
        asset_scope::allow_external_note_assets(app.handle(), &source).unwrap();
        let from = raw.find(href).unwrap();
        let note = super::import_note(
            app.handle().clone(),
            app.state(),
            destination.clone(),
            source.to_string_lossy().into_owned(),
            None,
            raw.clone(),
            vec![operations::ExternalNoteReference {
                from,
                to: from + href.len(),
                href: href.into(),
                html: false,
                pdf: false,
            }],
        )
        .await
        .unwrap();
        assert!(note.raw.contains(".helixnotes/attachments/"));
        assert_eq!(std::fs::read_to_string(source).unwrap(), raw);
        assert_eq!(std::fs::read(image).unwrap(), b"source image");
        let files: Vec<_> =
            std::fs::read_dir(std::path::Path::new(&destination).join(".helixnotes/attachments"))
                .unwrap()
                .collect();
        assert_eq!(files.len(), 1);
        assert_eq!(
            std::fs::read(files[0].as_ref().unwrap().path()).unwrap(),
            b"source image"
        );
    }

    #[tokio::test]
    async fn external_note_scope_does_not_allow_other_files_or_internal_metadata() {
        let root = tempfile::tempdir().unwrap();
        let vault = root.path().join("vault");
        let vault_path = vault.to_string_lossy().into_owned();
        crate::vault::operations::ensure_vault_structure(&vault_path).unwrap();
        let outside = root.path().join("outside.md");
        let non_markdown = root.path().join("outside.txt");
        let internal = vault.join(".helixnotes/history/snapshot.md");
        fs::create_dir_all(internal.parent().unwrap()).unwrap();
        for path in [&outside, &non_markdown, &internal] {
            fs::write(path, "Original").unwrap();
        }
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_fs::init())
            .manage(AppState::new(AppConfig {
                active_vault: Some(vault_path.clone()),
                ..Default::default()
            }))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let read = |path: &std::path::Path| {
            read_note(
                app.handle().clone(),
                app.state(),
                vault_path.clone(),
                path.to_string_lossy().into_owned(),
            )
        };
        assert!(read(&outside).await.is_err());
        for path in [&non_markdown, &internal] {
            app.fs_scope().allow_file(path).unwrap();
            assert!(read(path).await.is_err());
        }
        app.fs_scope().allow_file(&outside).unwrap();
        assert!(read_note(
            app.handle().clone(),
            app.state(),
            root.path().to_string_lossy().into_owned(),
            outside.to_string_lossy().into_owned()
        )
        .await
        .is_err());
        #[cfg(unix)]
        {
            let unselected = root.path().join("unselected.md");
            let link = vault.join("escape.md");
            fs::write(&unselected, "Private").unwrap();
            std::os::unix::fs::symlink(&unselected, &link).unwrap();
            assert!(read(&link).await.is_err());
        }
    }

    #[test]
    fn file_access_requires_the_active_vault_or_an_explicit_user_scope() {
        let root = std::env::temp_dir().join(format!(
            "helixnotes-external-access-{}",
            uuid::Uuid::new_v4()
        ));
        let vault = root.join("vault");
        let vault_file = vault.join("note.md");
        let outside_file = root.join("outside.txt");
        let outside_destination = root.join("export.txt");
        fs::create_dir_all(&vault).unwrap();
        fs::write(&vault_file, b"note").unwrap();
        fs::write(&outside_file, b"outside").unwrap();

        let config = AppConfig {
            active_vault: Some(vault.to_string_lossy().into_owned()),
            ..Default::default()
        };
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_fs::init())
            .manage(AppState::new(config))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();

        assert!(ensure_readable_path(app.handle(), &vault_file).is_ok());
        assert!(ensure_readable_path(app.handle(), &outside_file).is_err());
        assert!(ensure_writable_path(app.handle(), &outside_destination).is_err());

        app.fs_scope().allow_file(&outside_file).unwrap();
        app.fs_scope().allow_file(&outside_destination).unwrap();
        assert!(ensure_readable_path(app.handle(), &outside_file).is_ok());
        assert!(ensure_writable_path(app.handle(), &outside_destination).is_ok());

        fs::remove_dir_all(root).unwrap();
    }
}

/// File types that are handed to the system's default application. Opening a file takes one click
/// on a link in a note, and notes and attachments arrive through sync, import and shared vaults.
/// Anything the system might run rather than display (scripts, executables, shortcuts, installers,
/// HTML, files without an extension) is shown in the file manager instead, under its real name.
const OPENABLE_EXTENSIONS: &[&str] = &[
    "txt", "md", "markdown", "log", "csv", "tsv", "json", "xml", "yaml", "yml", "toml", "pdf",
    "rtf", "doc", "docx", "odt", "xls", "xlsx", "ods", "ppt", "pptx", "odp", "epub", "png", "jpg",
    "jpeg", "gif", "webp", "bmp", "tif", "tiff", "heic", "heif", "avif", "ico", "mp3", "wav",
    "flac", "ogg", "oga", "opus", "m4a", "aac", "mp4", "m4v", "mkv", "mov", "webm", "avi", "zip",
    "7z", "tar", "gz", "tgz", "bz2", "xz", "rar", "ics", "vcf",
];

fn opens_with_default_application(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            OPENABLE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
        })
}

#[tauri::command]
pub fn open_file(app: AppHandle, path: String) -> Result<(), String> {
    ensure_readable_path(&app, Path::new(&path))?;
    if opens_with_default_application(Path::new(&path)) {
        open_path_with_system(Path::new(&path))
    } else {
        tauri_plugin_opener::reveal_item_in_dir(path).map_err(|error| error.to_string())
    }
}

#[tauri::command]
pub fn reveal_file(app: AppHandle, path: String) -> Result<(), String> {
    ensure_readable_path(&app, Path::new(&path))?;
    tauri_plugin_opener::reveal_item_in_dir(path).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    validate_external_url(&url)?;
    open_url_with_system(&url)
}

#[tauri::command]
pub fn copy_file_to(app: AppHandle, source: String, destination: String) -> Result<(), String> {
    ensure_readable_path(&app, Path::new(&source))?;
    ensure_writable_path(&app, Path::new(&destination))?;
    std::fs::copy(&source, &destination).map_err(|e| format!("Failed to copy file: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn write_bytes_to(app: AppHandle, destination: String, data: Vec<u8>) -> Result<(), String> {
    ensure_writable_path(&app, Path::new(&destination))?;
    std::fs::write(&destination, &data).map_err(|e| format!("Failed to write file: {}", e))?;
    Ok(())
}

// ── Backup ──

#[tauri::command]
pub fn create_backup(app: AppHandle, vault_path: String) -> Result<(), String> {
    let (backup_dir, include_attachments, max_count, bookmark_id) = {
        let state = app.state::<AppState>();
        state.validate_vault(&vault_path)?;
        let config = state.config.lock().map_err(|e| e.to_string())?;
        let bookmark_id = config
            .vaults
            .iter()
            .find(|vault| vault.path == vault_path)
            .and_then(|vault| vault.bookmark_id.clone());
        (
            crate::backup::get_backup_dir(&config.backup_location)?,
            config.backup_include_attachments,
            config.backup_max_count,
            bookmark_id,
        )
    };

    std::thread::spawn(move || {
        use tauri::Emitter;
        let state = app.state::<AppState>();
        let _maintenance = state.vault_maintenance.blocking_lock();
        let result = {
            let _writes = state.note_save.blocking_lock();
            crate::backup::create_backup(&vault_path, &backup_dir, include_attachments)
        };
        match result {
            Ok(entry) => {
                if let Ok(mut config) = state.config.lock() {
                    let mut next = config.clone();
                    if let Some(vault) = next.vaults.iter_mut().find(|candidate| {
                        vault_matches_identity(candidate, &vault_path, bookmark_id.as_deref())
                    }) {
                        vault.last_backup_time = Some(entry.created.clone());
                        match save_app_config(&next) {
                            Ok(()) => *config = next,
                            Err(error) => {
                                log::error!("Could not persist backup timestamp: {error}")
                            }
                        }
                    }
                }
                if let Err(error) =
                    crate::backup::cleanup_old_backups(&backup_dir, &vault_path, max_count)
                {
                    log::error!("Could not prune completed backups: {error}");
                }
                let _ = app.emit(
                    "backup-done",
                    serde_json::json!({
                        "vault_path": vault_path, "bookmark_id": bookmark_id,
                        "success": true, "entry": entry,
                    }),
                );
            }
            Err(error) => {
                let _ = app.emit(
                    "backup-done",
                    serde_json::json!({
                        "vault_path": vault_path, "bookmark_id": bookmark_id,
                        "success": false, "error": error,
                    }),
                );
            }
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn list_backups(
    state: State<'_, AppState>,
    vault_path: String,
) -> Result<Vec<BackupEntry>, String> {
    state.validate_vault(&vault_path)?;
    let backup_dir = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        crate::backup::get_backup_dir(&config.backup_location)?
    };
    tauri::async_runtime::spawn_blocking(move || {
        crate::backup::list_backups(&backup_dir, &vault_path)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn restore_backup(
    app: AppHandle,
    vault_path: String,
    backup_path: String,
) -> Result<(), String> {
    let backup_dir = {
        let state = app.state::<AppState>();
        state.validate_vault(&vault_path)?;
        let config = state.config.lock().map_err(|e| e.to_string())?;
        crate::backup::get_backup_dir(&config.backup_location)?
    };

    std::thread::spawn(move || {
        use std::sync::atomic::Ordering;
        use tauri::Emitter;
        let state = app.state::<AppState>();
        let _maintenance = state.vault_maintenance.blocking_lock();
        let result = {
            let _writes = state.note_save.blocking_lock();
            state.importing.store(true, Ordering::Release);
            let result = crate::backup::restore_backup(&vault_path, &backup_dir, &backup_path)
                // Still under the maintenance lock, so no sync can run in between.
                .and_then(|()| crate::sync::forget_baseline(&vault_path));
            state.importing.store(false, Ordering::Release);
            result
        };
        match result {
            Ok(()) => {
                if let Ok(Some(search)) = state.vault_snapshot(&vault_path) {
                    if let Err(error) = search.queue_vault_refresh(&vault_path) {
                        log::error!("Could not refresh restored vault search: {error}");
                    }
                }
                let _ = app.emit(
                    "file-changed",
                    FileEvent {
                        vault_path: vault_path.clone(),
                        event_type: "modify".into(),
                        path: vault_path.clone(),
                        note: None,
                    },
                );
                let _ = app.emit(
                    "vault-restored",
                    serde_json::json!({ "vault_path": vault_path }),
                );
                let _ = app.emit(
                    "restore-done",
                    serde_json::json!({
                        "vault_path": vault_path, "success": true,
                    }),
                );
            }
            Err(error) => {
                let _ = app.emit(
                    "restore-done",
                    serde_json::json!({
                        "vault_path": vault_path, "success": false, "error": error,
                    }),
                );
            }
        }
    });
    Ok(())
}

#[tauri::command]
pub fn delete_backup(
    state: State<'_, AppState>,
    vault_path: String,
    backup_path: String,
) -> Result<(), String> {
    state.validate_vault(&vault_path)?;
    let backup_dir = {
        let config = state.config.lock().map_err(|error| error.to_string())?;
        crate::backup::get_backup_dir(&config.backup_location)?
    };
    crate::backup::delete_backup(&backup_dir, &backup_path)
}

#[tauri::command]
pub fn set_backup_settings(
    state: State<'_, AppState>,
    enabled: bool,
    frequency: String,
    max_count: u32,
    location: Option<String>,
    include_attachments: bool,
) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    config.backup_enabled = enabled;
    config.backup_frequency = frequency;
    config.backup_max_count = max_count;
    config.backup_location = location;
    config.backup_include_attachments = include_attachments;
    save_app_config(&config)?;
    Ok(())
}

// ── Version History ──

#[tauri::command]
pub async fn get_note_versions(
    state: State<'_, AppState>,
    vault_path: String,
    note_id: String,
) -> Result<Vec<crate::types::VersionEntry>, String> {
    state.validate_vault(&vault_path)?;
    let read_guard = state.note_save.clone().lock_owned().await;
    tauri::async_runtime::spawn_blocking(move || {
        let _read_guard = read_guard;
        crate::history::list_versions(&vault_path, &note_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn create_version(
    state: State<'_, AppState>,
    vault_path: String,
    path: String,
    note_id: String,
) -> Result<(), String> {
    state.validate_vault(&vault_path)?;
    let max_versions = state
        .config
        .lock()
        .map_err(|error| error.to_string())?
        .max_versions_per_note;
    let save_guard = state.note_save.clone().lock_owned().await;
    tauri::async_runtime::spawn_blocking(move || {
        let _save_guard = save_guard;
        let raw = operations::read_vault_note(&vault_path, &path)?.raw;
        crate::history::force_snapshot(&vault_path, &note_id, &raw, max_versions)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn get_note_version_content(
    state: State<'_, AppState>,
    vault_path: String,
    note_id: String,
    timestamp: String,
) -> Result<String, String> {
    state.validate_vault(&vault_path)?;
    let read_guard = state.note_save.clone().lock_owned().await;
    tauri::async_runtime::spawn_blocking(move || {
        let _read_guard = read_guard;
        crate::history::get_version(&vault_path, &note_id, &timestamp)
    })
    .await
    .map_err(|error| error.to_string())?
}

// ── AI ──

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn set_ai_settings(
    state: State<'_, AppState>,
    provider: Option<String>,
    api_key: Option<String>,
    model: String,
    writing_style: Option<String>,
    base_url: Option<String>,
    ollama_api_key: Option<String>,
    openai_compatible_base_url: Option<String>,
    openai_compatible_api_key: Option<String>,
) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    let key = api_key.filter(|k| !k.is_empty());
    match provider.as_deref() {
        Some("openai") => config.openai_api_key = key,
        Some("ollama") => {
            config.ollama_base_url = base_url.filter(|u| !u.trim().is_empty());
            config.ollama_api_key = ollama_api_key.filter(|k| !k.is_empty());
        }
        Some("openai_compatible") => {
            config.openai_compatible_base_url =
                openai_compatible_base_url.filter(|u| !u.trim().is_empty());
            config.openai_compatible_api_key = openai_compatible_api_key.filter(|k| !k.is_empty());
        }
        _ => config.ai_api_key = key,
    }
    config.ai_provider = provider;
    config.ai_model = model;
    config.ai_writing_style = writing_style.filter(|s| !s.trim().is_empty());
    save_app_config(&config)?;
    Ok(())
}

#[tauri::command]
pub fn test_ai_connection(app: AppHandle) -> Result<(), String> {
    let (provider, api_key, model, base_url) = {
        let state = app.state::<AppState>();
        let config = state.config.lock().map_err(|e| e.to_string())?;
        let provider = config
            .ai_provider
            .clone()
            .unwrap_or_else(|| "anthropic".to_string());
        let key = match provider.as_str() {
            "ollama" => Some(config.ollama_api_key.clone().unwrap_or_default()),
            "openai_compatible" => {
                Some(config.openai_compatible_api_key.clone().unwrap_or_default())
            }
            "openai" => config.openai_api_key.clone(),
            _ => config.ai_api_key.clone(),
        }
        .ok_or("No API key configured")?;
        let model = config.ai_model.clone();
        let base_url = match provider.as_str() {
            "openai_compatible" => config.openai_compatible_base_url.clone(),
            _ => config.ollama_base_url.clone(),
        };
        (provider, key, model, base_url)
    };

    std::thread::spawn(move || {
        use tauri::Emitter;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(crate::ai::test_connection(
            &provider,
            &api_key,
            &model,
            base_url.as_deref(),
        ));
        match result {
            Ok(msg) => {
                let _ = app.emit(
                    "ai-test-result",
                    serde_json::json!({ "success": true, "message": msg }),
                );
            }
            Err(e) => {
                let _ = app.emit(
                    "ai-test-result",
                    serde_json::json!({ "success": false, "error": e }),
                );
            }
        }
    });
    Ok(())
}

// ── Sync (WebDAV) ──

fn vault_matches_identity(vault: &VaultConfig, path: &str, bookmark_id: Option<&str>) -> bool {
    if let Some(bookmark_id) = bookmark_id {
        vault.bookmark_id.as_deref() == Some(bookmark_id)
    } else {
        vault.bookmark_id.is_none() && vault.path == path
    }
}

fn active_vault_index(config: &AppConfig) -> Result<usize, String> {
    let active = config.active_vault.as_deref().ok_or("No active vault")?;
    config
        .vaults
        .iter()
        .position(|vault| {
            vault_matches_identity(vault, active, config.active_bookmark_id.as_deref())
        })
        .ok_or_else(|| "Active vault not found in config".to_string())
}

fn active_vault_config(config: &AppConfig) -> Result<&VaultConfig, String> {
    Ok(&config.vaults[active_vault_index(config)?])
}

#[cfg(test)]
mod vault_identity_tests {
    use super::*;

    #[test]
    fn bookmark_identity_disambiguates_vaults_with_the_same_path() {
        let mut config = AppConfig {
            active_vault: Some("/same/path".to_string()),
            vaults: vec![
                VaultConfig {
                    path: "/same/path".to_string(),
                    name: "Local".to_string(),
                    ..Default::default()
                },
                VaultConfig {
                    path: "/same/path".to_string(),
                    name: "Files".to_string(),
                    bookmark_id: Some("bookmark".to_string()),
                    ..Default::default()
                },
            ],
            active_bookmark_id: Some("bookmark".to_string()),
            ..Default::default()
        };
        assert_eq!(active_vault_config(&config).unwrap().name, "Files");

        config.active_bookmark_id = None;
        assert_eq!(active_vault_config(&config).unwrap().name, "Local");
    }

    #[test]
    fn the_folder_request_setting_reaches_the_sync_and_older_configs_get_the_default() {
        let mut config = AppConfig {
            active_vault: Some("/v".to_string()),
            vaults: vec![VaultConfig {
                path: "/v".to_string(),
                name: "v".to_string(),
                sync_provider: Some("webdav".to_string()),
                webdav_url: Some("https://x.test/dav".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(sync_config_from(&config).unwrap().listing_parallelism, 0);
        config.vaults[0].sync_folder_requests = 3;
        assert_eq!(sync_config_from(&config).unwrap().listing_parallelism, 3);

        // A vault saved before the setting existed has no such field.
        let older: VaultConfig = serde_json::from_str(r#"{"path":"/v","name":"v"}"#).unwrap();
        assert_eq!(older.sync_folder_requests, 0);
    }
}

fn sync_config_from(config: &AppConfig) -> Result<crate::sync::WebdavConfig, String> {
    let v = active_vault_config(config)?;
    if v.sync_provider.as_deref() != Some("webdav") {
        return Err("Sync is not configured".to_string());
    }
    let url = v
        .webdav_url
        .clone()
        .filter(|u| !u.trim().is_empty())
        .ok_or("WebDAV URL is not set")?;
    Ok(crate::sync::WebdavConfig {
        url,
        username: v.webdav_username.clone().unwrap_or_default(),
        password: v.webdav_password.clone().unwrap_or_default(),
        listing_parallelism: v.sync_folder_requests as usize,
    })
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn set_sync_settings(
    state: State<'_, AppState>,
    provider: Option<String>,
    url: Option<String>,
    username: Option<String>,
    password: Option<String>,
    sync_on_open: bool,
    sync_on_change: bool,
    sync_interval_minutes: u32,
    sync_folder_requests: u32,
) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    let active_index = active_vault_index(&config)?;
    let v = &mut config.vaults[active_index];
    v.sync_provider = provider.filter(|p| !p.is_empty());
    v.webdav_url = url.filter(|u| !u.trim().is_empty());
    v.webdav_username = username.filter(|u| !u.is_empty());
    v.webdav_password = password.filter(|p| !p.is_empty());
    v.sync_on_open = sync_on_open;
    v.sync_on_change = sync_on_change;
    v.sync_interval_minutes = sync_interval_minutes;
    v.sync_folder_requests =
        crate::sync::effective_listing_parallelism(sync_folder_requests as usize) as u32;
    save_app_config(&config)?;
    Ok(())
}

#[tauri::command]
pub fn test_sync_connection(app: AppHandle) -> Result<(), String> {
    let cfg = {
        let state = app.state::<AppState>();
        let config = state.config.lock().map_err(|e| e.to_string())?;
        sync_config_from(&config)?
    };
    std::thread::spawn(move || {
        use tauri::Emitter;
        match crate::sync::test_connection(cfg) {
            Ok(msg) => {
                let _ = app.emit(
                    "sync-test-result",
                    serde_json::json!({ "success": true, "message": msg }),
                );
            }
            Err(e) => {
                let _ = app.emit(
                    "sync-test-result",
                    serde_json::json!({ "success": false, "error": e }),
                );
            }
        }
    });
    Ok(())
}

#[tauri::command]
pub fn sync_now(app: AppHandle) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    // Guard against overlapping syncs (manual button + interval + on-change can collide).
    if app.state::<AppState>().syncing.swap(true, Ordering::SeqCst) {
        return Ok(()); // a sync is already running
    }
    let (vault, bookmark_id, cfg) = {
        let state = app.state::<AppState>();
        let config = match state.config.lock() {
            Ok(c) => c,
            Err(e) => {
                state.syncing.store(false, Ordering::SeqCst);
                return Err(e.to_string());
            }
        };
        let gathered = config
            .active_vault
            .clone()
            .ok_or_else(|| "No active vault".to_string())
            .and_then(|vault| {
                sync_config_from(&config).map(|cfg| (vault, config.active_bookmark_id.clone(), cfg))
            });
        match gathered {
            Ok(vault_config) => vault_config,
            Err(e) => {
                drop(config);
                state.syncing.store(false, Ordering::SeqCst);
                return Err(e);
            }
        }
    };
    std::thread::spawn(move || {
        use tauri::Emitter;
        let result = crate::sync::run_sync(app.clone(), vault.clone(), cfg);
        app.state::<AppState>()
            .syncing
            .store(false, Ordering::SeqCst);
        match result {
            Ok(summary) => {
                let ts = chrono::Utc::now().to_rfc3339();
                if let Ok(mut config) = app.state::<AppState>().config.lock() {
                    if let Some(vault_config) = config.vaults.iter_mut().find(|candidate| {
                        vault_matches_identity(candidate, &vault, bookmark_id.as_deref())
                    }) {
                        vault_config.last_sync_time = Some(ts.clone());
                    }
                    let _ = save_app_config(&config);
                }
                let _ = app.emit(
                    "sync-done",
                    serde_json::json!({ "vault_path": vault, "bookmark_id": bookmark_id, "success": true, "summary": summary, "last_sync_time": ts }),
                );
            }
            Err(e) => {
                let _ = app.emit(
                    "sync-error",
                    serde_json::json!({ "vault_path": vault, "success": false, "error": e }),
                );
            }
        }
    });
    Ok(())
}

#[tauri::command]
pub fn ai_ask(
    app: AppHandle,
    action: String,
    text: String,
    custom_prompt: Option<String>,
    request_id: String,
) -> Result<(), String> {
    let (provider, api_key, model, writing_style, base_url) = {
        let state = app.state::<AppState>();
        let config = state.config.lock().map_err(|e| e.to_string())?;
        let provider = config
            .ai_provider
            .clone()
            .unwrap_or_else(|| "anthropic".to_string());
        let key = match provider.as_str() {
            "ollama" => Some(config.ollama_api_key.clone().unwrap_or_default()),
            "openai_compatible" => {
                Some(config.openai_compatible_api_key.clone().unwrap_or_default())
            }
            "openai" => config.openai_api_key.clone(),
            _ => config.ai_api_key.clone(),
        }
        .ok_or("No API key configured. Go to Settings > AI to set up your API key.")?;
        let model = config.ai_model.clone();
        let style = config.ai_writing_style.clone();
        let base_url = match provider.as_str() {
            "openai_compatible" => config.openai_compatible_base_url.clone(),
            _ => config.ollama_base_url.clone(),
        };
        (provider, key, model, style, base_url)
    };

    let mut system_prompt = "You are a helpful writing assistant inside a note-taking app called HelixNotes. \
        You help users improve, rewrite, summarize, and transform their text. \
        Return ONLY the resulting text - no explanations, no markdown code fences, no preamble. \
        Preserve the original language of the text unless specifically asked to translate. \
        Preserve any markdown formatting (bold, italic, links, images, tables, etc.) unless the user asks to change it. \
        If the text contains placeholders like __MEDIA_0__, __MEDIA_1__, etc., keep them exactly as they are in their original positions - they represent images and embedded files.".to_string();

    if let Some(ref style) = writing_style {
        system_prompt.push_str(&format!(
            "\n\nThe user's preferred writing style: {}",
            style
        ));
    }

    let user_message = match action.as_str() {
        "improve" => format!("Improve the writing quality of this text while keeping the same meaning and tone:\n\n{}", text),
        "fix_grammar" => format!("Fix all grammar, spelling, and punctuation errors in this text. Keep the original style:\n\n{}", text),
        "shorter" => format!("Make this text more concise while keeping the key points:\n\n{}", text),
        "longer" => format!("Expand this text with more detail while keeping the same style and tone:\n\n{}", text),
        "professional" => format!("Rewrite this text in a professional, formal tone:\n\n{}", text),
        "friendly" => format!("Rewrite this text in a casual, friendly tone:\n\n{}", text),
        "summarize" => format!("Write a brief summary of this text:\n\n{}", text),
        "explain" => format!("Explain this text in simpler terms:\n\n{}", text),
        "translate_en" => format!("Translate this text to English:\n\n{}", text),
        "translate_nl" => format!("Translate this text to Dutch:\n\n{}", text),
        "translate_de" => format!("Translate this text to German:\n\n{}", text),
        "translate_fr" => format!("Translate this text to French:\n\n{}", text),
        "translate_es" => format!("Translate this text to Spanish:\n\n{}", text),
        "custom" => {
            let prompt = custom_prompt.unwrap_or_else(|| "Improve this text".to_string());
            format!("{}\n\n{}", prompt, text)
        }
        _ => format!("Improve this text:\n\n{}", text),
    };

    crate::ai::ai_request(
        app,
        provider,
        api_key,
        model,
        system_prompt,
        user_message,
        request_id,
        base_url,
    );
    Ok(())
}

// ── Helpers ──

// On mobile (Android + iOS) the OS config dir is injected at startup via the Tauri
// path resolver, since dirs::config_dir() is not reliable in the app sandbox.
static MOBILE_CONFIG_DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

#[cfg(mobile)]
pub fn set_mobile_config_dir(path: std::path::PathBuf) {
    let _ = MOBILE_CONFIG_DIR.set(path);
}

fn app_config_path() -> Result<std::path::PathBuf, String> {
    // Prefer the injected mobile dir when present (set only on mobile); fall back to
    // the platform config dir on desktop.
    let app_dir = if let Some(mobile_dir) = MOBILE_CONFIG_DIR.get() {
        mobile_dir.join("helixnotes")
    } else if let Some(config_dir) = dirs::config_dir() {
        config_dir.join("helixnotes")
    } else {
        return Err("Config directory not available yet".to_string());
    };
    std::fs::create_dir_all(&app_dir).map_err(|e| e.to_string())?;
    Ok(app_dir.join("config.json"))
}

pub fn load_app_config() -> AppConfig {
    let mut config: AppConfig = app_config_path()
        .ok()
        .map(|path| read_app_config(&path))
        .unwrap_or_default();
    let migrated_sync = migrate_global_sync_to_vault(&mut config);
    let migrated_font = adopt_font_size_as_preference(&mut config);
    if migrated_sync || migrated_font {
        let _ = save_app_config(&config);
    }
    config
}

/// Reads the configuration, falling back to the defaults when there is none. A file that exists but
/// cannot be parsed is first copied aside: the defaults are saved over it on the next change, and
/// it holds the vault list, sync credentials, API keys and custom themes.
fn read_app_config(path: &Path) -> AppConfig {
    let Ok(text) = std::fs::read_to_string(path) else {
        return AppConfig::default();
    };
    match serde_json::from_str(&text) {
        Ok(config) => config,
        Err(error) => {
            if !text.trim().is_empty() {
                keep_unreadable_config(path, &text, &error.to_string());
            }
            AppConfig::default()
        }
    }
}

fn keep_unreadable_config(path: &Path, text: &str, reason: &str) {
    let Some(folder) = path.parent() else { return };
    // One copy per distinct content: loading again before anything is saved must not pile them up.
    let already_kept = std::fs::read_dir(folder).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("config.unreadable-")
                && std::fs::read_to_string(entry.path()).is_ok_and(|kept| kept == text)
        })
    });
    if already_kept {
        return;
    }
    let copy = folder.join(format!(
        "config.unreadable-{}.json",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    ));
    match write_private_file(&copy, text.as_bytes()) {
        Ok(()) => eprintln!(
            "Configuration could not be read ({reason}); a copy was kept at {}",
            copy.display()
        ),
        Err(error) => eprintln!(
            "Configuration could not be read ({reason}), and no copy could be kept: {error}"
        ),
    }
}

// One-time migration: the size in use when the preference was introduced becomes the preference,
// so resetting the zoom does not fall back to the built-in default for existing users.
fn adopt_font_size_as_preference(config: &mut AppConfig) -> bool {
    if config.preferred_font_size.is_some() || config.font_size.is_none() {
        return false;
    }
    config.preferred_font_size = config.font_size;
    true
}

// One-time migration: WebDAV sync moved from global AppConfig to per-vault VaultConfig.
// The old global settings go to the active vault if it has none yet, and are then cleared. Left in
// place, they were copied again at every start into whichever vault was active and had sync
// disabled, undoing "Disabled" and pointing a second vault at the first one's remote folder.
fn migrate_global_sync_to_vault(config: &mut AppConfig) -> bool {
    if config.sync_provider.is_none() && config.webdav_url.is_none() {
        return false;
    }
    let Ok(active_index) = active_vault_index(config) else {
        return false;
    };
    let g_provider = config.sync_provider.take();
    let g_url = config.webdav_url.take();
    let g_user = config.webdav_username.take();
    let g_pass = config.webdav_password.take();
    let g_on_open = std::mem::take(&mut config.sync_on_open);
    let g_on_change = std::mem::take(&mut config.sync_on_change);
    let g_interval = std::mem::take(&mut config.sync_interval_minutes);
    let g_last = config.last_sync_time.take();
    let v = &mut config.vaults[active_index];
    if v.sync_provider.is_none() && v.webdav_url.is_none() {
        v.sync_provider = g_provider;
        v.webdav_url = g_url;
        v.webdav_username = g_user;
        v.webdav_password = g_pass;
        v.sync_on_open = g_on_open;
        v.sync_on_change = g_on_change;
        v.sync_interval_minutes = g_interval;
        v.last_sync_time = g_last;
    }
    true
}

/// Writes an owner-only file by replacing it in one step: the data goes to a temporary file in the
/// same folder, is flushed to disk and then renamed over the target. A crash or a full disk during
/// the write leaves the previous file intact instead of an empty or half-written one.
fn write_private_file(path: &std::path::Path, data: &[u8]) -> Result<(), String> {
    use std::io::Write;

    let folder = path.parent().ok_or("File has no parent directory")?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".helixnotes-config-")
        .tempfile_in(folder)
        .map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    temporary
        .write_all(data)
        .map_err(|error| error.to_string())?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    temporary
        .persist(path)
        .map(|_| ())
        .map_err(|error| error.error.to_string())
}

#[cfg(all(test, unix))]
mod config_permission_tests {
    use super::write_private_file;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn config_files_are_owner_only() {
        let path = std::env::temp_dir().join(format!(
            "helixnotes-config-permissions-{}.json",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        write_private_file(&path, b"secret").unwrap();

        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"secret");
        std::fs::remove_file(path).unwrap();
    }
}

fn save_app_config(config: &AppConfig) -> Result<(), String> {
    let path = app_config_path()?;
    save_app_config_to(config, &path)
}

fn save_app_config_to(config: &AppConfig, path: &Path) -> Result<(), String> {
    let data = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    write_private_file(path, data.as_bytes())?;
    Ok(())
}

// ── Install Type Detection ──

#[tauri::command]
pub fn is_mobile_platform() -> bool {
    // Compile-time platform: true only for the Android/iOS builds. Authoritative, unlike the
    // webview user-agent, which some desktop WebKitGTK builds report mobile-looking (issue #63).
    cfg!(mobile)
}

#[tauri::command]
pub fn get_install_type() -> String {
    // Build-time override for distro packagers (e.g. Solus): build with
    // HELIXNOTES_INSTALL_TYPE=solus to report that type and suppress the in-app updater.
    if let Some(forced) = option_env!("HELIXNOTES_INSTALL_TYPE") {
        if !forced.is_empty() {
            return forced.to_string();
        }
    }

    // On mobile (iOS/Android) the concept of install type is irrelevant and the
    // Linux detection below would attempt std::process::Command which is forbidden
    // in the iOS sandbox. Return early.
    if cfg!(mobile) {
        return "mobile".to_string();
    }

    if cfg!(target_os = "macos") {
        "macos".to_string()
    } else if cfg!(target_os = "windows") {
        "windows".to_string()
    } else if std::path::Path::new("/var/lib/dpkg/info/helix-notes.list").exists() {
        "deb".to_string()
    } else if std::path::Path::new("/var/lib/pacman/local").exists()
        && ["helixnotes", "helixnotes-bin", "helixnotes-appimage-bin"]
            .iter()
            .any(|pkg| {
                std::process::Command::new("pacman")
                    .args(["-Q", pkg])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
            })
    {
        "aur".to_string()
    } else if std::env::var("APPIMAGE").is_ok() {
        "appimage".to_string()
    } else {
        "native".to_string()
    }
}

#[cfg(test)]
mod config_resilience_tests {
    use super::*;

    fn config_from(json: &str) -> (tempfile::TempDir, AppConfig) {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("config.json");
        std::fs::write(&path, json).unwrap();
        let config = read_app_config(&path);
        (folder, config)
    }

    fn kept_copies(folder: &Path) -> Vec<String> {
        std::fs::read_dir(folder)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.starts_with("config.unreadable-"))
            .collect()
    }

    #[test]
    fn missing_keys_and_damaged_entries_do_not_blank_the_configuration() {
        // No `theme`, a custom theme without its colors, a vault without a path, a toolbar of the wrong shape.
        let (folder, config) = config_from(
            r##"{"vaults":[{"path":"/notes","name":"Notes","webdav_url":"https://dav.example/x"},{"name":"broken"}],
                "active_vault":"/notes","font_size":18,"toolbar_layout":"oops",
                "custom_themes":[{"id":"a","name":"A","is_dark":true},
                  {"id":"b","name":"B","is_dark":false,"colors":{"bg_primary":"#fff","bg_secondary":"#fff","bg_tertiary":"#fff","bg_hover":"#fff","bg_active":"#fff","bg_editor":"#fff","text_primary":"#000","text_secondary":"#000","border_color":"#000"}}]}"##,
        );
        assert_eq!(config.vaults.len(), 1);
        assert_eq!(
            config.vaults[0].webdav_url.as_deref(),
            Some("https://dav.example/x")
        );
        assert_eq!(config.active_vault.as_deref(), Some("/notes"));
        assert_eq!(config.font_size, Some(18));
        assert_eq!(config.theme, AppConfig::default().theme);
        assert!(config.toolbar_layout.is_none());
        assert_eq!(config.custom_themes.len(), 1);
        assert_eq!(config.custom_themes[0].id, "b");
        assert!(kept_copies(folder.path()).is_empty());
    }

    #[test]
    fn an_unreadable_configuration_is_kept_aside_once() {
        let (folder, config) =
            config_from(r#"{"vaults":[{"path":"/notes","name":"Notes"}],"active_va"#);
        assert!(config.vaults.is_empty());
        let copies = kept_copies(folder.path());
        assert_eq!(copies.len(), 1);
        assert_eq!(
            std::fs::read_to_string(folder.path().join(&copies[0])).unwrap(),
            r#"{"vaults":[{"path":"/notes","name":"Notes"}],"active_va"#
        );
        // Loading again before anything is saved keeps no second copy.
        read_app_config(&folder.path().join("config.json"));
        assert_eq!(kept_copies(folder.path()).len(), 1);
        // No file and an empty file are a fresh start, not damage.
        assert!(read_app_config(&folder.path().join("absent.json"))
            .vaults
            .is_empty());
        let (empty, _) = config_from("");
        assert!(kept_copies(empty.path()).is_empty());
    }

    #[test]
    fn saving_replaces_the_file_in_one_step_and_leaves_nothing_behind() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("config.json");
        std::fs::write(&path, "previous").unwrap();
        save_app_config_to(&AppConfig::default(), &path).unwrap();
        assert!(read_app_config(&path).vaults.is_empty());
        let names: Vec<_> = std::fs::read_dir(folder.path())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("config.json")]);
    }

    fn vault(path: &str) -> crate::types::VaultConfig {
        serde_json::from_value(serde_json::json!({ "path": path, "name": path })).unwrap()
    }

    #[test]
    fn global_sync_settings_move_to_the_active_vault_once() {
        let mut config = AppConfig {
            vaults: vec![vault("/a"), vault("/b")],
            active_vault: Some("/a".into()),
            sync_provider: Some("webdav".into()),
            webdav_url: Some("https://dav.example/a".into()),
            webdav_username: Some("me".into()),
            webdav_password: Some("secret".into()),
            sync_on_open: true,
            ..AppConfig::default()
        };
        assert!(migrate_global_sync_to_vault(&mut config));
        assert_eq!(
            config.vaults[0].webdav_url.as_deref(),
            Some("https://dav.example/a")
        );
        assert!(config.vaults[0].sync_on_open);
        // The global copy is gone, so nothing is left to hand out again.
        assert!(config.sync_provider.is_none() && config.webdav_url.is_none());
        assert!(config.webdav_username.is_none() && config.webdav_password.is_none());
        assert!(!config.sync_on_open);

        // Disabling sync on that vault stays disabled, and another vault inherits nothing.
        config.vaults[0].sync_provider = None;
        config.vaults[0].webdav_url = None;
        config.active_vault = Some("/b".into());
        assert!(!migrate_global_sync_to_vault(&mut config));
        assert!(config.vaults[0].webdav_url.is_none());
        assert!(config.vaults[1].webdav_url.is_none());
    }

    #[test]
    fn a_vault_with_its_own_sync_settings_keeps_them_and_the_global_copy_is_cleared() {
        let mut own = vault("/a");
        own.webdav_url = Some("https://dav.example/own".into());
        let mut config = AppConfig {
            vaults: vec![own],
            active_vault: Some("/a".into()),
            webdav_url: Some("https://dav.example/old".into()),
            ..AppConfig::default()
        };
        assert!(migrate_global_sync_to_vault(&mut config));
        assert_eq!(
            config.vaults[0].webdav_url.as_deref(),
            Some("https://dav.example/own")
        );
        assert!(config.webdav_url.is_none());
    }
}

#[cfg(test)]
mod open_file_tests {
    use super::opens_with_default_application;
    use std::path::Path;

    #[test]
    fn only_document_and_media_types_go_to_the_default_application() {
        for name in [
            "report.pdf",
            "Photo.JPG",
            "notes.md",
            "data.csv",
            "clip.mp4",
            "archive.tar.gz",
            "a b/c d.docx",
        ] {
            assert!(opens_with_default_application(Path::new(name)), "{name}");
        }
        // Anything the system could run, and anything unknown, is revealed in the file manager.
        for name in [
            "report.cmd",
            "report.bat",
            "setup.exe",
            "run.ps1",
            "x.vbs",
            "x.js",
            "x.lnk",
            "x.url",
            "x.msi",
            "x.scr",
            "x.hta",
            "x.jar",
            "run.sh",
            "app.desktop",
            "x.AppImage",
            "x.command",
            "x.app",
            "x.pkg",
            "page.html",
            "drawing.svg",
            "macro.docm",
            "noextension",
            "report.pdf.",
            "report.pdf.exe",
            ".hidden",
            "weird.xyz",
        ] {
            assert!(!opens_with_default_application(Path::new(name)), "{name}");
        }
    }
}

#[cfg(test)]
mod font_size_preference_tests {
    use super::*;

    #[test]
    fn current_size_becomes_the_preference_once() {
        let mut config = AppConfig {
            font_size: Some(18),
            ..AppConfig::default()
        };
        assert!(adopt_font_size_as_preference(&mut config));
        assert_eq!(config.preferred_font_size, Some(18));
        // A later zoom changes only the size in use.
        config.font_size = Some(22);
        assert!(!adopt_font_size_as_preference(&mut config));
        assert_eq!(config.preferred_font_size, Some(18));
    }

    #[test]
    fn nothing_is_adopted_without_a_saved_size() {
        let mut config = AppConfig::default();
        assert!(!adopt_font_size_as_preference(&mut config));
        assert_eq!(config.preferred_font_size, None);
    }
}

#[cfg(test)]
mod vault_runtime_tests {
    use super::*;
    use std::sync::{Arc, TryLockError};
    use std::time::{Duration, Instant};

    fn activate(state: &AppState, path: &str, index: Arc<SearchIndex>) {
        let mut search = state.search_index.lock().unwrap();
        let mut config = state.config.lock().unwrap();
        *search = Some(index);
        config.active_vault = Some(path.to_string());
    }

    fn wait_for_search_lock(state: &AppState) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match state.search_index.try_lock() {
                Err(TryLockError::WouldBlock) => return,
                Err(TryLockError::Poisoned(error)) => panic!("{error}"),
                Ok(_) => {}
            }
            assert!(
                Instant::now() < deadline,
                "runtime operation did not acquire search before config"
            );
            std::thread::yield_now();
        }
    }

    #[test]
    fn reopening_a_vault_reuses_the_index_retained_by_background_work() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("A").to_string_lossy().into_owned();
        let b = root.path().join("B").to_string_lossy().into_owned();
        operations::ensure_vault_structure(&a).unwrap();
        operations::ensure_vault_structure(&b).unwrap();
        let note = operations::create_note(&a, None, "Alpha").unwrap();
        let state = AppState::new(AppConfig::default());
        let first = state.search_index_for_vault(&a).unwrap();
        first.rebuild(&a).unwrap();
        activate(&state, &a, first.clone());

        let background_work = first;
        let second = state.search_index_for_vault(&b).unwrap();
        activate(&state, &b, second);
        let reopened = state.search_index_for_vault(&a).unwrap();
        assert_eq!(
            reopened.search("Alpha", 10, 0).unwrap().results[0].path,
            note.path
        );
        activate(&state, &a, reopened.clone());
        drop(background_work);
        state.search_index.lock().unwrap().take();
        drop(reopened);

        // Expired weak entries do not retain a writer or prevent a later open.
        let reopened_after_shutdown = state.search_index_for_vault(&a).unwrap();
        assert_eq!(
            reopened_after_shutdown
                .search("Alpha", 10, 0)
                .unwrap()
                .results[0]
                .path,
            note.path
        );
    }

    #[test]
    fn reindex_snapshot_cannot_mix_vaults_during_a_switch() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("A").to_string_lossy().into_owned();
        let b = root.path().join("B").to_string_lossy().into_owned();
        operations::ensure_vault_structure(&a).unwrap();
        operations::ensure_vault_structure(&b).unwrap();
        let alpha = operations::create_note(&a, None, "Alpha").unwrap();
        let beta = operations::create_note(&b, None, "Beta").unwrap();
        let state = Arc::new(AppState::new(AppConfig::default()));
        let first = state.search_index_for_vault(&a).unwrap();
        let second = state.search_index_for_vault(&b).unwrap();
        first.rebuild(&a).unwrap();
        second.rebuild(&b).unwrap();
        activate(&state, &a, first.clone());

        // Stop the snapshot at its config read, then queue a switch behind it.
        let config = state.config.lock().unwrap();
        let snapshot_state = state.clone();
        let snapshot = std::thread::spawn(move || snapshot_state.active_search().unwrap());
        wait_for_search_lock(&state);
        let switch_state = state.clone();
        let switch_index = second.clone();
        let switch = std::thread::spawn(move || activate(&switch_state, &b, switch_index));
        drop(config);
        let (index, vault_path) = snapshot.join().unwrap();
        switch.join().unwrap();
        index.rebuild(&vault_path).unwrap();

        assert_eq!(
            first.search("Alpha", 10, 0).unwrap().results[0].path,
            alpha.path
        );
        assert!(first.search("Beta", 10, 0).unwrap().results.is_empty());
        assert_eq!(
            second.search("Beta", 10, 0).unwrap().results[0].path,
            beta.path
        );
    }

    #[test]
    fn removing_a_vault_preserves_concurrent_settings_and_rolls_back_write_failure() {
        let root = tempfile::tempdir().unwrap();
        let vault = root.path().join("vault").to_string_lossy().into_owned();
        operations::ensure_vault_structure(&vault).unwrap();
        let app = tauri::test::mock_builder()
            .manage(AppState::new(AppConfig {
                active_vault: Some(vault.clone()),
                vaults: vec![VaultConfig {
                    path: vault.clone(),
                    name: "Vault".into(),
                    ..VaultConfig::default()
                }],
                ..AppConfig::default()
            }))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let state = app.state::<AppState>();
        let index = state.search_index_for_vault(&vault).unwrap();
        *state.search_index.lock().unwrap() = Some(index.clone());

        assert!(
            remove_vault_path(app.handle(), &state, vault.clone(), None, root.path(),).is_err()
        );
        let (retained_index, retained_path) = state.active_search().unwrap();
        assert!(Arc::ptr_eq(&retained_index, &index));
        assert_eq!(retained_path, vault);

        let config_path = root.path().join("app-config.json");
        let saved_path = config_path.clone();
        let handle = app.handle().clone();
        let mut config = state.config.lock().unwrap();
        let removal = std::thread::spawn(move || {
            let state = handle.state::<AppState>();
            remove_vault_path(&handle, &state, vault, None, &config_path)
        });
        wait_for_search_lock(&state);
        config.theme = "nord".into();
        drop(config);
        removal.join().unwrap().unwrap();

        assert!(state.search_index.lock().unwrap().is_none());
        let saved: AppConfig = serde_json::from_slice(&std::fs::read(saved_path).unwrap()).unwrap();
        assert_eq!(saved.theme, "nord");
        assert!(saved.active_vault.is_none());
        assert!(saved.vaults.is_empty());
        assert_eq!(state.config.lock().unwrap().theme, "nord");
    }
}

#[cfg(test)]
mod mutation_order_tests {
    use super::*;
    use futures::FutureExt;

    #[tokio::test]
    async fn concurrent_editors_reject_stale_baselines_and_accept_the_saved_revision() {
        let vault = tempfile::tempdir().unwrap();
        let root = vault.path().to_string_lossy().into_owned();
        operations::ensure_vault_structure(&root).unwrap();
        let entry = operations::create_note(&root, None, "Shared").unwrap();
        let initial = operations::read_note(&root, &entry.path).unwrap();
        let app = tauri::test::mock_builder()
            .manage(AppState::new(AppConfig {
                active_vault: Some(root.clone()),
                ..AppConfig::default()
            }))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let state = app.state::<AppState>();
        let held = state.note_save.lock().await;
        let mut first = Box::pin(save_note(
            app.state(),
            root.clone(),
            entry.path.clone(),
            initial.meta.clone(),
            "# Shared\n\nFirst editor's content".into(),
            initial.raw.clone(),
        ));
        let mut stale = Box::pin(save_note(
            app.state(),
            root.clone(),
            entry.path.clone(),
            initial.meta.clone(),
            "# Shared\n\nStale editor's content".into(),
            initial.raw,
        ));
        assert!(first.as_mut().now_or_never().is_none());
        assert!(stale.as_mut().now_or_never().is_none());
        drop(held);
        let accepted = first.await.unwrap();
        assert!(stale.await.is_err());
        assert_eq!(std::fs::read_to_string(&entry.path).unwrap(), accepted.raw);
        assert_eq!(accepted.content, "# Shared\n\nFirst editor's content");
        let subsequent = save_note(
            app.state(),
            root.clone(),
            entry.path.clone(),
            accepted.meta,
            "# Shared\n\nNext accepted edit".into(),
            accepted.raw,
        )
        .await
        .unwrap();
        assert_eq!(subsequent.content, "# Shared\n\nNext accepted edit");
        assert_eq!(
            std::fs::read_to_string(&entry.path).unwrap(),
            subsequent.raw
        );
    }

    #[tokio::test]
    async fn detached_history_quick_access_titles_and_graph_keep_their_origin() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let a = first
            .path()
            .join("AlphaVault")
            .to_string_lossy()
            .into_owned();
        let b = second
            .path()
            .join("BetaVault")
            .to_string_lossy()
            .into_owned();
        operations::ensure_vault_structure(&a).unwrap();
        operations::ensure_vault_structure(&b).unwrap();
        let alpha = operations::create_note(&a, None, "Alpha").unwrap();
        let beta = operations::create_note(&b, None, "Beta").unwrap();
        let alpha_raw = operations::read_note(&a, &alpha.path).unwrap().raw;
        operations::add_quick_access(&b, "Beta.md").unwrap();
        // The same ID may legitimately exist in a copied vault. Its history
        // must still be looked up using the window's explicit vault origin.
        crate::history::force_snapshot(&b, &alpha.meta.id, "Beta history", 20).unwrap();
        let app = tauri::test::mock_builder()
            .manage(AppState::new(AppConfig {
                active_vault: Some(a.clone()),
                vaults: vec![
                    VaultConfig {
                        path: a.clone(),
                        ..VaultConfig::default()
                    },
                    VaultConfig {
                        path: b.clone(),
                        ..VaultConfig::default()
                    },
                ],
                ..AppConfig::default()
            }))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let state = app.state::<AppState>();
        let held = state.note_save.lock().await;
        let mut snapshot = Box::pin(create_version(
            app.state(),
            a.clone(),
            alpha.path.clone(),
            alpha.meta.id.clone(),
        ));
        let mut adding = Box::pin(add_quick_access(app.state(), a.clone(), "Alpha.md".into()));
        assert!(snapshot.as_mut().now_or_never().is_none());
        assert!(adding.as_mut().now_or_never().is_none());
        state.config.lock().unwrap().active_vault = Some(b.clone());
        drop(held);
        snapshot.await.unwrap();
        adding.await.unwrap();

        let versions = get_note_versions(app.state(), a.clone(), alpha.meta.id.clone())
            .await
            .unwrap();
        assert_eq!(versions.len(), 1);
        let restored = get_note_version_content(
            app.state(),
            a.clone(),
            alpha.meta.id.clone(),
            versions[0].timestamp.clone(),
        )
        .await
        .unwrap();
        assert_eq!(restored, alpha_raw);
        let beta_versions = get_note_versions(app.state(), b.clone(), alpha.meta.id.clone())
            .await
            .unwrap();
        assert_eq!(
            get_note_version_content(
                app.state(),
                b.clone(),
                alpha.meta.id,
                beta_versions[0].timestamp.clone(),
            )
            .await
            .unwrap(),
            "Beta history"
        );

        let shortcuts = get_quick_access(app.state(), a.clone()).await.unwrap();
        assert_eq!(
            shortcuts
                .iter()
                .map(|note| note.path.as_str())
                .collect::<Vec<_>>(),
            [alpha.path.as_str()]
        );
        let titles = get_all_note_titles(app.state(), a.clone()).await.unwrap();
        assert_eq!(
            titles
                .iter()
                .map(|note| note.title.as_str())
                .collect::<Vec<_>>(),
            ["Alpha"]
        );
        let graph = get_graph_data(app.state(), a.clone()).await.unwrap();
        assert_eq!(
            graph
                .nodes
                .iter()
                .map(|note| note.path.as_str())
                .collect::<Vec<_>>(),
            [alpha.path.as_str()]
        );
        remove_quick_access(app.state(), a.clone(), "Alpha.md".into())
            .await
            .unwrap();
        assert!(get_quick_access(app.state(), a).await.unwrap().is_empty());
        let beta_shortcuts = get_quick_access(app.state(), b).await.unwrap();
        assert_eq!(
            beta_shortcuts
                .iter()
                .map(|note| note.path.as_str())
                .collect::<Vec<_>>(),
            [beta.path.as_str()]
        );
    }

    #[tokio::test]
    async fn queued_writes_and_detached_saves_keep_the_authorized_origin() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let a = first.path().to_string_lossy().into_owned();
        let b = second.path().to_string_lossy().into_owned();
        operations::ensure_vault_structure(&a).unwrap();
        operations::ensure_vault_structure(&b).unwrap();
        let alpha = operations::create_note(&a, None, "Shared").unwrap();
        let beta = operations::create_note(&b, None, "Shared").unwrap();
        let beta_before = std::fs::read(&beta.path).unwrap();
        let app = tauri::test::mock_builder()
            .manage(AppState::new(AppConfig {
                active_vault: Some(a.clone()),
                vaults: vec![
                    VaultConfig {
                        path: a.clone(),
                        ..VaultConfig::default()
                    },
                    VaultConfig {
                        path: b.clone(),
                        ..VaultConfig::default()
                    },
                ],
                ..AppConfig::default()
            }))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let state = app.state::<AppState>();
        let held = state.note_save.lock().await;
        let mut creating = Box::pin(create_note(app.state(), a.clone(), None, "Queued".into()));
        let mut saving = Box::pin(save_note(
            app.state(),
            a.clone(),
            alpha.path.clone(),
            alpha.meta.clone(),
            "# Shared\n\nQueued save".into(),
            std::fs::read_to_string(&alpha.path).unwrap(),
        ));
        assert!(creating.as_mut().now_or_never().is_none());
        assert!(saving.as_mut().now_or_never().is_none());
        state.config.lock().unwrap().active_vault = Some(b.clone());
        // Authorization fails before entering the write queue, not after disk access.
        let rejected = create_note(
            app.state(),
            outside.path().to_string_lossy().into_owned(),
            None,
            "Forbidden".into(),
        )
        .now_or_never()
        .unwrap();
        assert!(rejected.is_err());
        drop(held);
        let created = creating.await.unwrap();
        saving.await.unwrap();
        assert_eq!(Path::new(&created.path).parent(), Some(first.path()));
        assert!(!second.path().join("Queued.md").exists());
        assert_eq!(
            operations::read_note(&a, &alpha.path).unwrap().content,
            "# Shared\n\nQueued save"
        );
        assert_eq!(std::fs::read(&beta.path).unwrap(), beta_before);

        // A configured but inactive origin is valid for a later detached-window save.
        save_note(
            app.state(),
            a.clone(),
            alpha.path.clone(),
            alpha.meta,
            "# Shared\n\nDetached save".into(),
            std::fs::read_to_string(&alpha.path).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(
            read_note(
                app.handle().clone(),
                app.state(),
                a.clone(),
                alpha.path.clone()
            )
            .await
            .unwrap()
            .content,
            "# Shared\n\nDetached save"
        );
        assert!(
            read_note(app.handle().clone(), app.state(), b.clone(), alpha.path)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(&beta.path).unwrap(), beta_before);
    }

    #[tokio::test]
    async fn icon_imports_and_field_patches_serialize_with_cleanup_and_notebook_moves() {
        let vault = tempfile::tempdir().unwrap();
        let path = vault.path().to_string_lossy().into_owned();
        operations::ensure_vault_structure(&path).unwrap();
        operations::create_notebook(&path, None, "Projects").unwrap();
        operations::create_notebook(&path, Some("Projects"), "Child").unwrap();
        operations::create_notebook(&path, None, "Archive").unwrap();
        let app = tauri::test::mock_builder()
            .manage(AppState::new(AppConfig {
                active_vault: Some(path.clone()),
                ..AppConfig::default()
            }))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let state = app.state::<AppState>();
        let held = state.note_save.lock().await;
        let mut color = Box::pin(set_tag_style(
            app.state(),
            path.clone(),
            "Work".into(),
            TagStyleChange::Color(Some("#AABBCC".into())),
        ));
        let mut tag_import = Box::pin(import_tag_icon(
            app.state(),
            path.clone(),
            "work".into(),
            "same.png".into(),
            vec![1, 2, 3],
        ));
        let mut notebook_import = Box::pin(import_notebook_icon(
            app.state(),
            path.clone(),
            "Projects/Child".into(),
            "same.png".into(),
            vec![4, 5, 6],
        ));
        let mut scan = Box::pin(find_orphaned_attachments(app.state(), path.clone()));
        assert!(color.as_mut().now_or_never().is_none());
        assert!(tag_import.as_mut().now_or_never().is_none());
        assert!(notebook_import.as_mut().now_or_never().is_none());
        assert!(scan.as_mut().now_or_never().is_none());
        drop(held);
        assert_eq!(
            color.await.unwrap().unwrap().color.as_deref(),
            Some("#aabbcc")
        );
        let tag = tag_import.await.unwrap();
        let icon = notebook_import.await.unwrap();
        assert_eq!(tag.color.as_deref(), Some("#aabbcc"));
        assert!(scan.await.unwrap().is_empty());
        let tag_icon = tag.icon.unwrap();
        assert_ne!(icon, tag_icon);
        assert_eq!(std::fs::read(vault.path().join(&icon)).unwrap(), [4, 5, 6]);
        assert_eq!(
            std::fs::read(vault.path().join(&tag_icon)).unwrap(),
            [1, 2, 3]
        );
        let recolored = set_tag_style(
            app.state(),
            path.clone(),
            "WORK".into(),
            TagStyleChange::Color(Some("#112233".into())),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(recolored.icon.as_deref(), Some(tag_icon.as_str()));
        assert_eq!(recolored.color.as_deref(), Some("#112233"));
        assert_eq!(
            get_tag_styles(app.state(), path.clone()).await.unwrap()["Work"],
            recolored
        );

        let held = state.note_save.lock().await;
        let mut changing = Box::pin(set_notebook_icon(
            app.state(),
            path.clone(),
            "Projects".into(),
            Some("builtin:briefcase".into()),
        ));
        let mut renaming = Box::pin(rename_notebook(
            app.handle().clone(),
            app.state(),
            path.clone(),
            vault.path().join("Projects").to_string_lossy().into_owned(),
            "Renamed".into(),
        ));
        assert!(changing.as_mut().now_or_never().is_none());
        assert!(renaming.as_mut().now_or_never().is_none());
        drop(held);
        changing.await.unwrap();
        let renamed = renaming.await.unwrap();
        move_notebook(
            app.handle().clone(),
            app.state(),
            path.clone(),
            renamed,
            vault.path().join("Archive").to_string_lossy().into_owned(),
        )
        .await
        .unwrap();
        let icons = get_notebook_icons(app.state(), path.clone()).await.unwrap();
        assert_eq!(icons.get("Archive/Renamed/Child"), Some(&icon));
        assert_eq!(
            icons.get("Archive/Renamed").map(String::as_str),
            Some("builtin:briefcase")
        );
        assert!(!icons.contains_key("Projects/Child"));
        let names = [&icon, &tag_icon]
            .map(|relative| {
                Path::new(relative)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .to_vec();
        assert_eq!(
            trash_orphaned_attachments(app.state(), path.clone(), names)
                .await
                .unwrap(),
            0
        );

        state
            .syncing
            .store(true, std::sync::atomic::Ordering::Release);
        assert!(find_orphaned_attachments(app.state(), path.clone())
            .await
            .is_err());
        assert!(trash_orphaned_attachments(app.state(), path, Vec::new())
            .await
            .is_err());
        state
            .syncing
            .store(false, std::sync::atomic::Ordering::Release);
    }

    #[tokio::test]
    async fn note_creation_and_save_work_without_an_initialized_search_index() {
        let vault = tempfile::tempdir().unwrap();
        let vault_path = vault.path().to_string_lossy().into_owned();
        operations::ensure_vault_structure(&vault_path).unwrap();
        let app = tauri::test::mock_builder()
            .manage(AppState::new(AppConfig {
                active_vault: Some(vault_path.clone()),
                ..AppConfig::default()
            }))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let note = create_note(app.state(), vault_path.clone(), None, "Draft".into())
            .await
            .unwrap();
        save_note(
            app.state(),
            vault_path.clone(),
            note.path.clone(),
            note.meta,
            "# Draft\n\nSaved before search is ready.".into(),
            std::fs::read_to_string(&note.path).unwrap(),
        )
        .await
        .unwrap();
        let saved = operations::read_note(&vault_path, &note.path).unwrap();
        assert_eq!(saved.meta.title, "Draft");
        assert_eq!(saved.content, "# Draft\n\nSaved before search is ready.");
    }

    #[tokio::test]
    async fn move_and_trash_wait_for_pending_saves_and_keep_the_latest_content() {
        let vault = tempfile::tempdir().unwrap();
        let vault_path = vault.path().to_string_lossy().into_owned();
        let archive = vault.path().join("Archive");
        std::fs::create_dir(&archive).unwrap();
        let entry = operations::create_note(&vault_path, None, "Note").unwrap();
        let app = tauri::test::mock_builder()
            .manage(AppState::new(AppConfig {
                active_vault: Some(vault_path.clone()),
                ..AppConfig::default()
            }))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let state = app.state::<AppState>();

        let saving = state.note_save.lock().await;
        let mut moving = Box::pin(move_note(
            app.handle().clone(),
            app.state(),
            vault_path.clone(),
            entry.path.clone(),
            archive.to_string_lossy().into_owned(),
        ));
        assert!(moving.as_mut().now_or_never().is_none());
        operations::save_note(
            &vault_path,
            &entry.path,
            &entry.meta,
            "Latest saved text",
            &std::fs::read_to_string(&entry.path).unwrap(),
        )
        .unwrap();
        drop(saving);
        let moved_path = moving.await.unwrap();
        assert!(!Path::new(&entry.path).exists());
        let moved = operations::read_note(&vault_path, &moved_path).unwrap();
        assert_eq!(moved.content, "# Note\n\nLatest saved text");

        let saving = state.note_save.lock().await;
        let mut deleting = Box::pin(delete_note(
            app.state(),
            vault_path.clone(),
            moved_path.clone(),
        ));
        assert!(deleting.as_mut().now_or_never().is_none());
        operations::save_note(
            &vault_path,
            &moved_path,
            &moved.meta,
            "Latest text before trash",
            &moved.raw,
        )
        .unwrap();
        drop(saving);
        deleting.await.unwrap();
        assert!(!Path::new(&moved_path).exists());
        let trash = operations::get_trash_contents(&vault_path).unwrap();
        assert_eq!(trash.notes.len(), 1);
        let raw = std::fs::read_to_string(&trash.notes[0].path).unwrap();
        let (_, content) = crate::vault::frontmatter::parse_note(&raw, "Note.md");
        assert_eq!(content, "# Note\n\nLatest text before trash");
    }
}
