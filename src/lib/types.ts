export interface NoteMeta {
  id: string;
  title: string;
  tags: string[];
  pinned: boolean;
  created: string;
  modified: string;
}

export interface NoteEntry {
  path: string;
  relative_path: string;
  meta: NoteMeta;
  preview: string;
}

export interface TrashNotebookEntry {
  name: string;
  path: string;
  note_count: number;
  modified: string;
}

export interface TrashContents {
  notes: NoteEntry[];
  notebooks: TrashNotebookEntry[];
}

export interface NotebookEntry {
  name: string;
  path: string;
  relative_path: string;
  children: NotebookEntry[];
  note_count: number;
}

export interface TagStyle {
  icon?: string | null;
  color?: string | null;
}

export type TagStyleChange =
  | { kind: "icon"; value: string | null }
  | { kind: "color"; value: string | null }
  | { kind: "reset" };

export interface NoteContent {
  path: string;
  meta: NoteMeta;
  content: string;
  raw: string;
}

export interface VaultConfig {
  path: string;
  name: string;
  bookmark_id?: string | null;
  // Per-vault WebDAV sync (moved off the global AppConfig).
  sync_provider?: string | null;
  webdav_url?: string | null;
  webdav_username?: string | null;
  webdav_password?: string | null;
  sync_on_open?: boolean;
  sync_on_change?: boolean;
  sync_interval_minutes?: number;
  sync_folder_requests?: number;
  last_sync_time?: string | null;
  last_backup_time?: string | null;
}

export interface PendingOpenFile {
  id: number;
  path: string;
}

export interface ExternalVaultResult {
  bookmarkId: string;
  path: string;
  name: string;
}

export interface CustomThemeColors {
  bg_primary: string;
  bg_secondary: string;
  bg_tertiary: string;
  bg_hover: string;
  bg_active: string;
  bg_editor: string;
  text_primary: string;
  text_secondary: string;
  border_color: string;
}

export interface CustomTheme {
  id: string;
  name: string;
  is_dark: boolean;
  colors: CustomThemeColors;
}

export type StartupView = "all" | "quickaccess" | "tasks" | "daily";

export interface ToolbarLayout {
  main: string[];
  more: string[];
}

export interface AppConfig {
  vaults: VaultConfig[];
  active_vault: string | null;
  active_bookmark_id?: string | null;
  theme: string;
  system_light_theme: string;
  system_dark_theme: string;
  accent_color: string | null;
  font_size: number | null;
  preferred_font_size?: number | null;
  scroll_to_change_font_size: boolean;
  toolbar_layout: ToolbarLayout | null;
  font_family: string | null;
  line_height: number | null;
  ui_scale: number | null;
  content_width: number | null;
  compact_notes: boolean;
  show_note_dates: boolean;
  show_note_switcher: boolean;
  time_format: string;
  week_start: string;
  daily_title_format: string;
  gpu_acceleration: boolean;
  autostart: boolean;
  pdf_preview: boolean;
  pdf_height: number;
  title_mode: string;
  hide_title_in_body: boolean;
  show_line_numbers: boolean;
  show_link_arrows: boolean;
  show_all_notes: boolean;
  show_quick_access: boolean;
  show_tasks: boolean;
  show_daily_notes: boolean;
  show_trash: boolean;
  backup_enabled: boolean;
  backup_frequency: string;
  backup_max_count: number;
  backup_location: string | null;
  backup_include_attachments: boolean;
  max_versions_per_note: number;
  ai_provider: string | null;
  ai_api_key: string | null;
  openai_api_key: string | null;
  ollama_base_url: string | null;
  ollama_api_key: string | null;
  openai_compatible_base_url: string | null;
  openai_compatible_api_key: string | null;
  ai_model: string;
  ai_writing_style: string | null;
  default_view_mode: boolean;
  new_notes_in_source_mode: boolean;
  show_tray_icon: boolean;
  close_to_tray: boolean;
  enable_wiki_links: boolean;
  startup_view: StartupView;
  restore_last_session: boolean;
  sync_provider: string | null;
  webdav_url: string | null;
  webdav_username: string | null;
  webdav_password: string | null;
  sync_on_open: boolean;
  sync_on_change: boolean;
  sync_interval_minutes: number;
  last_sync_time: string | null;
  custom_themes: CustomTheme[];
}

export interface VaultState {
  last_open_note: string | null;
  sidebar_width: number;
  notelist_width: number;
  outline_width?: number;
  sidebar_collapsed: boolean;
  notelist_collapsed: boolean;
  collapsed_notebooks: string[];
  notebook_sort_mode?: string;
  notebook_order?: Record<string, number>;
  note_order?: Record<string, number>;
  sort_mode?: string;
  group_notes_by_date?: boolean;
  last_view_mode?: string;
  last_notebook?: string | null;
  last_tag?: string | null;
  tasks_layout?: string;
  tasks_hide_completed?: boolean;
  tasks_only_flagged?: boolean;
  tasks_sort?: string;
}

export interface SearchMatch {
  start: number;
  end: number;
}

export interface SearchTarget {
  field: 'body' | 'title' | 'tags';
  start: number;
  end: number;
  text: string;
  before: string;
  after: string;
}

export interface SearchResult {
  path: string;
  title: string;
  snippet: string;
  score: number;
  title_matches: SearchMatch[];
  snippet_matches: SearchMatch[];
  match_target: SearchTarget | null;
}

export interface SearchPage {
  results: SearchResult[];
  total: number;
}

export interface FileEvent {
  vault_path: string;
  event_type: string;
  path: string;
  note?: NoteEntry | null;
}

export interface ImportResult {
  files_converted: number;
  links_converted: number;
  frontmatter_normalized: number;
  syntax_converted: number;
  attachments_moved: number;
}

export interface VaultStats {
  total_notes: number;
  total_attachments: number;
  notes_size: number;
  attachments_size: number;
  total_size: number;
}

export interface BackupEntry {
  filename: string;
  path: string;
  size: number;
  created: string;
  /** Made from a different vault folder; all vaults share one backup folder. */
  other_vault?: boolean;
}

export interface VersionEntry {
  timestamp: string;
  size: number;
}

export interface AiStreamEvent {
  request_id: string;
  event_type: string;
  text: string | null;
  error: string | null;
}

export interface NoteTitleEntry {
  title: string;
  path: string;
}

export type SortMode = "modified" | "title" | "created" | "custom";
export type ViewMode =
  | "all"
  | "notebook"
  | "tag"
  | "trash"
  | "search"
  | "quickaccess"
  | "daily"
  | "tasks";

export interface TaskItem {
  note_path: string;
  note_title: string;
  line: number;
  raw_line: string;
  text: string;
  completed: boolean;
  due: string | null;
  priority: string | null;
}
