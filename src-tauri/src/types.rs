use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteMeta {
    pub id: String,
    pub title: String,
    pub tags: Vec<String>,
    pub pinned: bool,
    pub created: DateTime<Utc>,
    pub modified: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteEntry {
    pub path: String,
    pub relative_path: String,
    pub meta: NoteMeta,
    pub preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrashContents {
    pub notes: Vec<NoteEntry>,
    pub notebooks: Vec<TrashNotebookEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrashNotebookEntry {
    pub name: String,
    pub path: String,
    pub note_count: usize,
    pub modified: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotebookEntry {
    pub name: String,
    pub path: String,
    pub relative_path: String,
    pub children: Vec<NotebookEntry>,
    pub note_count: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TagStyle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "lowercase")]
pub enum TagStyleChange {
    Icon(Option<String>),
    Color(Option<String>),
    Reset,
}

impl TagStyle {
    pub fn is_empty(&self) -> bool {
        self.icon
            .as_ref()
            .map(|value| value.trim().is_empty())
            .unwrap_or(true)
            && self
                .color
                .as_ref()
                .map(|value| value.trim().is_empty())
                .unwrap_or(true)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteContent {
    pub path: String,
    pub meta: NoteMeta,
    pub content: String,
    pub raw: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VaultConfig {
    pub path: String,
    pub name: String,
    #[serde(default)]
    pub bookmark_id: Option<String>,
    // Per-vault WebDAV sync (was previously global on AppConfig; those fields are kept deprecated for migration).
    #[serde(default)]
    pub sync_provider: Option<String>,
    #[serde(default)]
    pub webdav_url: Option<String>,
    #[serde(default)]
    pub webdav_username: Option<String>,
    #[serde(default)]
    pub webdav_password: Option<String>,
    #[serde(default)]
    pub sync_on_open: bool,
    #[serde(default)]
    pub sync_on_change: bool,
    #[serde(default)]
    pub sync_interval_minutes: u32,
    /// Folders to list on the server at once; 0 means the default.
    #[serde(default)]
    pub sync_folder_requests: u32,
    #[serde(default)]
    pub last_sync_time: Option<String>,
    #[serde(default)]
    pub last_backup_time: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomThemeColors {
    pub bg_primary: String,
    pub bg_secondary: String,
    pub bg_tertiary: String,
    pub bg_hover: String,
    pub bg_active: String,
    pub bg_editor: String,
    pub text_primary: String,
    pub text_secondary: String,
    pub border_color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomTheme {
    pub id: String,
    pub name: String,
    pub is_dark: bool,
    pub colors: CustomThemeColors,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StartupView {
    Daily,
    QuickAccess,
    Tasks,
    #[default]
    #[serde(other)]
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolbarLayout {
    pub main: Vec<String>,
    pub more: Vec<String>,
}

/// Keeps the entries of a list that still parse and drops the rest, so one damaged entry (a custom
/// theme missing a color, say) does not make the whole configuration unreadable.
fn lenient_list<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let values = Vec::<serde_json::Value>::deserialize(deserializer).unwrap_or_default();
    Ok(values
        .into_iter()
        .filter_map(|value| serde_json::from_value(value).ok())
        .collect())
}

/// An optional setting that no longer parses is treated as not set.
fn lenient_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = serde_json::Value::deserialize(deserializer).unwrap_or(serde_json::Value::Null);
    Ok(serde_json::from_value(value).ok())
}

// Missing keys take their default: a configuration written by an older version, or edited by
// hand, must still load instead of being replaced by a blank one.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    #[serde(deserialize_with = "lenient_list")]
    pub vaults: Vec<VaultConfig>,
    pub active_vault: Option<String>,
    #[serde(default)]
    pub active_bookmark_id: Option<String>,
    pub theme: String,
    /// Themes used when `theme` is "system": the frontend picks one by the OS color scheme.
    /// Default to the plain "light"/"dark" schemes so existing configs keep their behavior.
    #[serde(default = "default_system_light_theme")]
    pub system_light_theme: String,
    #[serde(default = "default_system_dark_theme")]
    pub system_dark_theme: String,
    #[serde(default)]
    pub accent_color: Option<String>,
    #[serde(default)]
    pub font_size: Option<u32>,
    /// The size chosen in Appearance. Zooming changes `font_size` only, and resetting the zoom
    /// returns to this.
    #[serde(default)]
    pub preferred_font_size: Option<u32>,
    #[serde(default = "default_true")]
    pub scroll_to_change_font_size: bool,
    #[serde(default, deserialize_with = "lenient_option")]
    pub toolbar_layout: Option<ToolbarLayout>,
    #[serde(default)]
    pub font_family: Option<String>,
    #[serde(default)]
    pub line_height: Option<f64>,
    #[serde(default)]
    pub ui_scale: Option<f64>,
    #[serde(default)]
    pub content_width: Option<u32>,
    #[serde(default)]
    pub compact_notes: bool,
    #[serde(default = "default_true")]
    pub show_note_dates: bool,
    #[serde(default)]
    pub show_note_switcher: bool,
    #[serde(default)]
    pub time_format: String,
    #[serde(default = "default_week_start")]
    pub week_start: String,
    #[serde(default = "default_daily_title_format")]
    pub daily_title_format: String,
    #[serde(default)]
    pub gpu_acceleration: bool,
    #[serde(default)]
    pub autostart: bool,
    #[serde(default)]
    pub pdf_preview: bool,
    #[serde(default = "default_pdf_height")]
    pub pdf_height: u32,
    #[serde(default = "default_title_mode")]
    pub title_mode: String,
    #[serde(default)]
    pub hide_title_in_body: bool,
    #[serde(default)]
    pub show_line_numbers: bool,
    #[serde(default = "default_true")]
    pub show_link_arrows: bool,
    #[serde(default = "default_true")]
    pub show_all_notes: bool,
    #[serde(default = "default_true")]
    pub show_quick_access: bool,
    #[serde(default = "default_true")]
    pub show_tasks: bool,
    #[serde(default = "default_true")]
    pub show_daily_notes: bool,
    #[serde(default = "default_true")]
    pub show_trash: bool,
    #[serde(default)]
    pub backup_enabled: bool,
    #[serde(default = "default_backup_frequency")]
    pub backup_frequency: String,
    #[serde(default = "default_backup_max")]
    pub backup_max_count: u32,
    #[serde(default)]
    pub backup_location: Option<String>,
    #[serde(default)]
    pub backup_include_attachments: bool,
    #[serde(default = "default_max_versions")]
    pub max_versions_per_note: u32,
    #[serde(default)]
    pub ai_provider: Option<String>,
    #[serde(default)]
    pub ai_api_key: Option<String>,
    #[serde(default)]
    pub openai_api_key: Option<String>,
    #[serde(default)]
    pub ollama_base_url: Option<String>,
    #[serde(default)]
    pub ollama_api_key: Option<String>,
    #[serde(default)]
    pub openai_compatible_base_url: Option<String>,
    #[serde(default)]
    pub openai_compatible_api_key: Option<String>,
    #[serde(default = "default_ai_model")]
    pub ai_model: String,
    #[serde(default)]
    pub ai_writing_style: Option<String>,
    #[serde(default)]
    pub default_view_mode: bool,
    #[serde(default)]
    pub new_notes_in_source_mode: bool,
    #[serde(default)]
    pub show_tray_icon: bool,
    #[serde(default)]
    pub close_to_tray: bool,
    #[serde(default = "default_true")]
    pub enable_wiki_links: bool,
    #[serde(default)]
    pub startup_view: StartupView,
    #[serde(default)]
    pub restore_last_session: bool,
    // DEPRECATED: WebDAV sync moved to per-vault VaultConfig. Kept for one release to migrate old configs.
    #[serde(default)]
    pub sync_provider: Option<String>,
    #[serde(default)]
    pub webdav_url: Option<String>,
    #[serde(default)]
    pub webdav_username: Option<String>,
    #[serde(default)]
    pub webdav_password: Option<String>,
    #[serde(default)]
    pub sync_on_open: bool,
    #[serde(default)]
    pub sync_on_change: bool,
    #[serde(default)]
    pub sync_interval_minutes: u32,
    #[serde(default)]
    pub last_sync_time: Option<String>,
    #[serde(default, deserialize_with = "lenient_list")]
    pub custom_themes: Vec<CustomTheme>,
}

fn default_true() -> bool {
    true
}

fn default_outline_width() -> f64 {
    220.0
}

fn default_pdf_height() -> u32 {
    600
}

fn default_backup_frequency() -> String {
    "24h".to_string()
}

fn default_backup_max() -> u32 {
    10
}

fn default_title_mode() -> String {
    "input".to_string()
}

fn default_week_start() -> String {
    "monday".to_string()
}

fn default_daily_title_format() -> String {
    "localized".to_string()
}

fn default_max_versions() -> u32 {
    20
}

fn default_ai_model() -> String {
    "claude-sonnet-5".to_string()
}

fn default_system_light_theme() -> String {
    "light".to_string()
}

fn default_system_dark_theme() -> String {
    "dark".to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            vaults: Vec::new(),
            active_vault: None,
            active_bookmark_id: None,
            theme: "system".to_string(),
            system_light_theme: default_system_light_theme(),
            system_dark_theme: default_system_dark_theme(),
            accent_color: None,
            font_size: None,
            preferred_font_size: None,
            scroll_to_change_font_size: true,
            toolbar_layout: None,
            font_family: None,
            line_height: None,
            ui_scale: None,
            content_width: None,
            compact_notes: false,
            show_note_dates: true,
            show_note_switcher: false,
            time_format: "relative".to_string(),
            week_start: "monday".to_string(),
            daily_title_format: "localized".to_string(),
            gpu_acceleration: true,
            autostart: false,
            pdf_preview: false,
            pdf_height: 600,
            title_mode: "input".to_string(),
            hide_title_in_body: false,
            show_line_numbers: false,
            show_link_arrows: true,
            show_all_notes: true,
            show_quick_access: true,
            show_tasks: true,
            show_daily_notes: true,
            show_trash: true,
            backup_enabled: false,
            backup_frequency: "24h".to_string(),
            backup_max_count: 10,
            backup_location: None,
            backup_include_attachments: false,
            max_versions_per_note: 20,
            ai_provider: None,
            ai_api_key: None,
            openai_api_key: None,
            ollama_base_url: None,
            ollama_api_key: None,
            openai_compatible_base_url: None,
            openai_compatible_api_key: None,
            ai_model: default_ai_model(),
            ai_writing_style: None,
            default_view_mode: false,
            new_notes_in_source_mode: false,
            show_tray_icon: false,
            close_to_tray: false,
            enable_wiki_links: true,
            startup_view: StartupView::All,
            restore_last_session: false,
            sync_provider: None,
            webdav_url: None,
            webdav_username: None,
            webdav_password: None,
            sync_on_open: false,
            sync_on_change: false,
            sync_interval_minutes: 0,
            last_sync_time: None,
            custom_themes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultState {
    pub last_open_note: Option<String>,
    pub sidebar_width: f64,
    pub notelist_width: f64,
    #[serde(default = "default_outline_width")]
    pub outline_width: f64,
    pub sidebar_collapsed: bool,
    #[serde(default)]
    pub notelist_collapsed: bool,
    #[serde(default)]
    pub collapsed_notebooks: Vec<String>,
    #[serde(default)]
    pub notebook_sort_mode: String,
    #[serde(default)]
    pub notebook_order: std::collections::HashMap<String, i32>,
    #[serde(default)]
    pub note_order: std::collections::HashMap<String, i32>,
    #[serde(default)]
    pub sort_mode: String,
    #[serde(default)]
    pub last_view_mode: String,
    #[serde(default)]
    pub last_notebook: Option<String>,
    #[serde(default)]
    pub last_tag: Option<String>,
    #[serde(default)]
    pub tasks_layout: String,
    #[serde(default = "default_true")]
    pub tasks_hide_completed: bool,
    #[serde(default)]
    pub tasks_only_flagged: bool,
    #[serde(default)]
    pub tasks_sort: String,
}

impl Default for VaultState {
    fn default() -> Self {
        Self {
            last_open_note: None,
            sidebar_width: 220.0,
            notelist_width: 280.0,
            outline_width: default_outline_width(),
            sidebar_collapsed: false,
            notelist_collapsed: false,
            collapsed_notebooks: Vec::new(),
            notebook_sort_mode: String::new(),
            notebook_order: std::collections::HashMap::new(),
            note_order: std::collections::HashMap::new(),
            sort_mode: String::new(),
            last_view_mode: String::new(),
            last_notebook: None,
            last_tag: None,
            tasks_layout: String::new(),
            tasks_hide_completed: true,
            tasks_only_flagged: false,
            tasks_sort: String::new(),
        }
    }
}

/// Half-open UTF-16 offsets into the corresponding plain-text result field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchMatch {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SearchTargetField {
    Body,
    Title,
    Tags,
}

/// Half-open UTF-16 offsets into the indexed field, with exact source context.
/// Body excludes frontmatter; tags use the indexed space-joined values.
/// Text is capped at 240 Unicode scalars and each context at 32 scalars.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchTarget {
    pub field: SearchTargetField,
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub path: String,
    pub title: String,
    pub snippet: String,
    pub score: f32,
    pub title_matches: Vec<SearchMatch>,
    pub snippet_matches: Vec<SearchMatch>,
    pub match_target: Option<SearchTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchPage {
    pub results: Vec<SearchResult>,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEvent {
    pub vault_path: String,
    pub event_type: String,
    pub path: String,
    pub note: Option<NoteEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    pub files_converted: u64,
    pub links_converted: u64,
    pub frontmatter_normalized: u64,
    pub syntax_converted: u64,
    pub attachments_moved: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultStats {
    pub total_notes: u64,
    pub total_attachments: u64,
    pub notes_size: u64,
    pub attachments_size: u64,
    pub total_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupEntry {
    pub filename: String,
    pub path: String,
    pub size: u64,
    pub created: String,
    /// The archive was made from a different vault folder. All vaults share one backup folder.
    #[serde(default)]
    pub other_vault: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionEntry {
    pub timestamp: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiStreamEvent {
    pub request_id: String,
    pub event_type: String, // "text", "done", "error"
    pub text: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteTitleEntry {
    pub title: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub title: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub source: usize,
    pub target: usize,
    pub bidirectional: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskItem {
    pub note_path: String,
    pub note_title: String,
    pub line: usize,
    pub raw_line: String,
    pub text: String,
    pub completed: bool,
    pub due: Option<String>,
    pub priority: Option<String>,
}

#[cfg(test)]
mod startup_view_tests {
    use super::{AppConfig, StartupView};

    #[test]
    fn serializes_supported_startup_views() {
        for (view, expected) in [
            (StartupView::All, "\"all\""),
            (StartupView::QuickAccess, "\"quickaccess\""),
            (StartupView::Tasks, "\"tasks\""),
            (StartupView::Daily, "\"daily\""),
        ] {
            assert_eq!(serde_json::to_string(&view).unwrap(), expected);
        }
    }

    #[test]
    fn unknown_startup_view_falls_back_to_all_notes() {
        assert_eq!(
            serde_json::from_str::<StartupView>("\"future-view\"").unwrap(),
            StartupView::All
        );
    }

    #[test]
    fn existing_configs_default_system_theme_pair() {
        let mut value = serde_json::to_value(AppConfig::default()).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("system_light_theme");
        object.remove("system_dark_theme");

        let config: AppConfig = serde_json::from_value(value).unwrap();

        assert_eq!(config.system_light_theme, "light");
        assert_eq!(config.system_dark_theme, "dark");
    }

    #[test]
    fn note_switcher_is_opt_in_for_new_and_existing_configs() {
        let config = AppConfig::default();
        assert!(!config.show_note_switcher);

        let mut value = serde_json::to_value(config).unwrap();
        value.as_object_mut().unwrap().remove("show_note_switcher");
        let config: AppConfig = serde_json::from_value(value).unwrap();

        assert!(!config.show_note_switcher);
    }

    #[test]
    fn scroll_font_sizing_stays_enabled_for_new_and_existing_configs() {
        let config = AppConfig::default();
        assert!(config.scroll_to_change_font_size);

        let mut value = serde_json::to_value(config).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("scroll_to_change_font_size");
        let config: AppConfig = serde_json::from_value(value).unwrap();

        assert!(config.scroll_to_change_font_size);
    }
}
