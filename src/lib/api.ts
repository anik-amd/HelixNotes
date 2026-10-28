import { invoke } from "@tauri-apps/api/core";
import type { ExternalNoteReference } from "./editor/externalNote";
import type {
  AppConfig,
  CustomTheme,
  NoteContent,
  NoteEntry,
  NoteMeta,
  NotebookEntry,
  NoteTitleEntry,
  SearchPage,
  TrashContents,
  VaultState,
  VaultStats,
  TagStyle,
  TagStyleChange,
  ImportResult,
  BackupEntry,
  VersionEntry,
  TaskItem,
  ExternalVaultResult,
  StartupView,
  ToolbarLayout,
  PendingOpenFile,
} from "./types";

export async function openVault(path: string): Promise<void> {
  return invoke("open_vault", { path });
}

export async function chooseExternalVault(): Promise<ExternalVaultResult | null> {
  return invoke("choose_external_vault");
}

export async function restoreExternalVault(bookmarkId: string): Promise<ExternalVaultResult> {
  return invoke("restore_external_vault", { bookmarkId });
}

export async function removeVault(path: string, bookmarkId?: string | null): Promise<void> {
  return invoke("remove_vault", { path, bookmarkId: bookmarkId ?? null });
}

export async function getAppConfig(): Promise<AppConfig> {
  return invoke("get_app_config");
}

export async function completeWindowClose(): Promise<void> {
  return invoke("complete_window_close");
}

export async function cancelWindowClose(): Promise<void> {
  return invoke("cancel_window_close");
}

export async function setTheme(theme: string): Promise<void> {
  return invoke("set_theme", { theme });
}

export async function setSystemThemes(light: string, dark: string): Promise<void> {
  return invoke("set_system_themes", { light, dark });
}

export async function setAccentColor(color: string): Promise<void> {
  return invoke("set_accent_color", { color });
}

export async function saveCustomTheme(theme: CustomTheme): Promise<void> {
  return invoke("save_custom_theme", { theme });
}

export async function deleteCustomTheme(id: string): Promise<void> {
  return invoke("delete_custom_theme", { id });
}

export async function exportCustomTheme(id: string, path: string): Promise<void> {
  return invoke("export_custom_theme", { id, path });
}

export async function importCustomThemes(path: string): Promise<CustomTheme[]> {
  return invoke("import_custom_themes", { path });
}

/** `preferred` marks a choice made in Appearance, which is what resetting the zoom returns to. */
export async function setFontSize(size: number, preferred = false): Promise<void> {
  return invoke("set_font_size", { size, preferred });
}

export async function setScrollToChangeFontSize(enabled: boolean): Promise<void> {
  return invoke("set_scroll_to_change_font_size", { enabled });
}

export async function setToolbarLayout(layout: ToolbarLayout | null): Promise<void> {
  return invoke("set_toolbar_layout", { layout });
}

export async function setFontFamily(family: string): Promise<void> {
  return invoke("set_font_family", { family });
}

export async function setLineHeight(height: number): Promise<void> {
  return invoke("set_line_height", { height });
}

export async function setUiScale(scale: number): Promise<void> {
  return invoke("set_ui_scale", { scale });
}

export async function setContentWidth(width: number | null): Promise<void> {
  return invoke("set_content_width", { width });
}

export async function getNotebooks(): Promise<NotebookEntry[]> {
  return invoke("get_notebooks");
}

export async function countRootNotes(): Promise<number> {
  return invoke("count_root_notes");
}

export async function createNotebook(
  vaultPath: string,
  parentRelative: string | null,
  name: string,
): Promise<NotebookEntry> {
  return invoke("create_notebook", { vaultPath, parentRelative, name });
}

export async function renameNotebook(
  vaultPath: string,
  path: string,
  newName: string,
): Promise<string> {
  return invoke("rename_notebook", { vaultPath, path, newName });
}

export async function deleteNotebook(vaultPath: string, path: string): Promise<void> {
  return invoke("delete_notebook", { vaultPath, path });
}

export async function moveNotebook(
  vaultPath: string,
  notebookPath: string,
  destParent: string,
): Promise<string> {
  return invoke("move_notebook", { vaultPath, notebookPath, destParent });
}

export async function getNotes(
  notebookPath: string | null,
): Promise<NoteEntry[]> {
  return invoke("get_notes", { notebookPath });
}

export async function readNote(vaultPath: string, path: string): Promise<NoteContent> {
  return invoke("read_note", { vaultPath, path });
}

export async function saveNote(
  vaultPath: string,
  path: string,
  meta: NoteMeta,
  body: string,
  expectedRaw: string,
): Promise<NoteContent> {
  return invoke("save_note", { vaultPath, path, meta, body, expectedRaw });
}

export async function importNote(
  vaultPath: string,
  sourcePath: string,
  notebookRelative: string | null,
  expectedRaw: string,
  references: ExternalNoteReference[],
): Promise<NoteContent> {
  return invoke("import_note", { vaultPath, sourcePath, notebookRelative, expectedRaw, references });
}

export async function createNote(
  vaultPath: string,
  notebookRelative: string | null,
  title: string,
): Promise<NoteEntry> {
  return invoke("create_note", { vaultPath, notebookRelative, title });
}

export async function duplicateNote(vaultPath: string, path: string): Promise<NoteEntry> {
  return invoke("duplicate_note", { vaultPath, path });
}

export async function createDailyNote(vaultPath: string, date?: string): Promise<NoteEntry> {
  return invoke("create_daily_note", { vaultPath, date: date ?? null });
}

export async function renameNote(
  vaultPath: string,
  path: string,
  newTitle: string,
): Promise<string> {
  return invoke("rename_note", { vaultPath, path, newTitle });
}

export async function deleteNote(vaultPath: string, path: string): Promise<void> {
  return invoke("delete_note", { vaultPath, path });
}

export async function moveNote(
  vaultPath: string,
  notePath: string,
  destNotebook: string,
): Promise<string> {
  return invoke("move_note", { vaultPath, notePath, destNotebook });
}

export async function getAllTags(): Promise<[string, number][]> {
  return invoke("get_all_tags");
}

export async function getAllNoteTitles(vaultPath: string): Promise<NoteTitleEntry[]> {
  return invoke("get_all_note_titles", { vaultPath });
}

export async function getNoteSwitcherTitles(
  recentPaths: string[],
): Promise<NoteTitleEntry[]> {
  return invoke("get_note_switcher_titles", { recentPaths });
}

export async function getGraphData(vaultPath: string): Promise<{ nodes: { title: string; path: string }[]; edges: { source: number; target: number; bidirectional: boolean }[] }> {
  return invoke("get_graph_data", { vaultPath });
}

export async function searchNotes(
  query: string,
  limit?: number,
  offset?: number,
): Promise<SearchPage> {
  return invoke("search_notes", { query, limit, offset });
}

export async function reindex(): Promise<void> {
  return invoke("reindex");
}

export async function getTrash(): Promise<TrashContents> {
  return invoke("get_trash");
}

export async function restoreNote(
  vaultPath: string,
  trashPath: string,
  destNotebook: string | null,
): Promise<string> {
  return invoke("restore_note", { vaultPath, trashPath, destNotebook });
}

export async function restoreNotebook(vaultPath: string, trashPath: string): Promise<string> {
  return invoke("restore_notebook", { vaultPath, trashPath });
}

export async function permanentDelete(vaultPath: string, path: string): Promise<void> {
  return invoke("permanent_delete", { vaultPath, path });
}

export async function emptyTrash(vaultPath: string): Promise<void> {
  return invoke("empty_trash", { vaultPath });
}

export async function loadVaultState(vaultPath: string): Promise<VaultState> {
  return invoke("load_vault_state", { vaultPath });
}

export async function saveVaultState(vaultPath: string, vaultState: VaultState): Promise<void> {
  return invoke("save_vault_state", { vaultPath, vaultState });
}

export async function copyTextToClipboard(text: string): Promise<void> {
  return invoke("copy_text_to_clipboard", { text });
}

export async function readClipboardImage(): Promise<number[]> {
  return invoke("read_clipboard_image");
}

export interface ClipboardContents {
  text: string | null;
  html: string | null;
}

export async function readClipboardContents(): Promise<ClipboardContents> {
  return invoke("read_clipboard_contents");
}

export async function copyImageToClipboard(path: string): Promise<void> {
  return invoke("copy_image_to_clipboard", { path });
}

export async function saveImage(vaultPath: string, name: string, data: number[]): Promise<string> {
  return invoke("save_image", { vaultPath, name, data });
}

export async function saveAttachment(
  vaultPath: string,
  name: string,
  data: number[],
): Promise<string> {
  return invoke("save_attachment", { vaultPath, name, data });
}

export async function getNotebookIcons(vaultPath: string): Promise<Record<string, string>> {
  return invoke("get_notebook_icons", { vaultPath });
}

export async function setNotebookIcon(
  vaultPath: string,
  notebookRelative: string,
  iconRelative: string | null,
): Promise<void> {
  return invoke("set_notebook_icon", { vaultPath, notebookRelative, iconRelative });
}

export async function getTagStyles(vaultPath: string): Promise<Record<string, TagStyle>> {
  return invoke("get_tag_styles", { vaultPath });
}

export async function setTagStyle(
  vaultPath: string,
  tag: string,
  change: TagStyleChange,
): Promise<TagStyle | null> {
  return invoke("set_tag_style", { vaultPath, tag, change });
}

export async function importNotebookIcon(
  vaultPath: string,
  notebookRelative: string,
  name: string,
  data: number[],
): Promise<string> {
  return invoke("import_notebook_icon", { vaultPath, notebookRelative, name, data });
}

export async function importTagIcon(
  vaultPath: string,
  tag: string,
  name: string,
  data: number[],
): Promise<TagStyle> {
  return invoke("import_tag_icon", { vaultPath, tag, name, data });
}

export async function setGeneralSettings(
  compactNotes: boolean,
  timeFormat: string,
  weekStart: string,
  dailyTitleFormat: string,
  gpuAcceleration: boolean,
  autostart: boolean,
  pdfPreview: boolean,
  pdfHeight: number,
  titleMode: string,
  hideTitleInBody: boolean,
  showLineNumbers: boolean,
  showLinkArrows: boolean,
  defaultViewMode: boolean,
  newNotesInSourceMode: boolean,
  showTrayIcon: boolean,
  closeToTray: boolean,
  enableWikiLinks: boolean,
  showNoteDates: boolean,
  showNoteSwitcher: boolean,
  startupView: StartupView,
  restoreLastSession: boolean,
  showAllNotes: boolean,
  showQuickAccess: boolean,
  showTasks: boolean,
  showDailyNotes: boolean,
  showTrash: boolean,
): Promise<void> {
  return invoke("set_general_settings", {
    compactNotes,
    timeFormat,
    weekStart,
    dailyTitleFormat,
    gpuAcceleration,
    autostart,
    pdfPreview,
    pdfHeight,
    titleMode,
    hideTitleInBody,
    showLineNumbers,
    showLinkArrows,
    defaultViewMode,
    newNotesInSourceMode,
    showTrayIcon,
    closeToTray,
    enableWikiLinks,
    showNoteDates,
    showNoteSwitcher,
    startupView,
    restoreLastSession,
    showAllNotes,
    showQuickAccess,
    showTasks,
    showDailyNotes,
    showTrash,
  });
}

export async function getQuickAccess(vaultPath: string): Promise<NoteEntry[]> {
  return invoke("get_quick_access", { vaultPath });
}

export async function addQuickAccess(vaultPath: string, noteRelative: string): Promise<void> {
  return invoke("add_quick_access", { vaultPath, noteRelative });
}

export async function removeQuickAccess(vaultPath: string, noteRelative: string): Promise<void> {
  return invoke("remove_quick_access", { vaultPath, noteRelative });
}

export async function reorderQuickAccess(paths: string[]): Promise<void> {
  return invoke("reorder_quick_access", { paths });
}

export async function getVaultStats(): Promise<VaultStats> {
  return invoke("get_vault_stats");
}

export interface OrphanAttachment {
  name: string;
  size: number;
}

export async function findOrphanedAttachments(vaultPath: string): Promise<OrphanAttachment[]> {
  return invoke("find_orphaned_attachments", { vaultPath });
}

export async function trashOrphanedAttachments(vaultPath: string, names: string[]): Promise<number> {
  return invoke("trash_orphaned_attachments", { vaultPath, names });
}

export async function importObsidian(): Promise<void> {
  return invoke("import_obsidian");
}

export async function openFile(path: string): Promise<void> {
  return invoke("open_file", { path });
}

export async function revealFile(path: string): Promise<void> {
  return invoke("reveal_file", { path });
}

export async function openUrl(url: string): Promise<void> {
  return invoke("open_url", { url });
}

export async function copyFileTo(
  source: string,
  destination: string,
): Promise<void> {
  return invoke("copy_file_to", { source, destination });
}

export async function writeBytesTo(
  destination: string,
  data: Uint8Array,
): Promise<void> {
  return invoke("write_bytes_to", { destination, data: Array.from(data) });
}

export async function copyPngToClipboard(data: Uint8Array): Promise<void> {
  return invoke("copy_png_to_clipboard", { data: Array.from(data) });
}

// ── Backup ──

export async function createBackup(vaultPath: string): Promise<void> {
  return invoke("create_backup", { vaultPath });
}

export async function listBackups(vaultPath: string): Promise<BackupEntry[]> {
  return invoke("list_backups", { vaultPath });
}

export async function restoreBackup(vaultPath: string, backupPath: string): Promise<void> {
  return invoke("restore_backup", { vaultPath, backupPath });
}

export async function deleteBackup(vaultPath: string, backupPath: string): Promise<void> {
  return invoke("delete_backup", { vaultPath, backupPath });
}

export async function setBackupSettings(
  enabled: boolean,
  frequency: string,
  maxCount: number,
  location: string | null,
  includeAttachments: boolean,
): Promise<void> {
  return invoke("set_backup_settings", {
    enabled,
    frequency,
    maxCount,
    location,
    includeAttachments,
  });
}

// ── Tasks ──

export async function getTasks(notePath?: string): Promise<TaskItem[]> {
  return invoke("get_tasks", { notePath: notePath ?? null });
}

export async function setTaskDone(
  vaultPath: string,
  notePath: string,
  line: number,
  rawLine: string,
  done: boolean,
): Promise<void> {
  return invoke("set_task_done", { vaultPath, notePath, line, rawLine, done });
}

export async function setTaskPriority(
  vaultPath: string,
  notePath: string,
  line: number,
  rawLine: string,
  priority: string | null,
): Promise<void> {
  return invoke("set_task_priority", { vaultPath, notePath, line, rawLine, priority });
}

export async function setTaskDue(
  vaultPath: string,
  notePath: string,
  line: number,
  rawLine: string,
  due: string | null,
): Promise<void> {
  return invoke("set_task_due", { vaultPath, notePath, line, rawLine, due });
}

// ── Sync (WebDAV) ──

export async function setSyncSettings(
  provider: string | null,
  url: string | null,
  username: string | null,
  password: string | null,
  syncOnOpen: boolean,
  syncOnChange: boolean,
  syncIntervalMinutes: number,
  syncFolderRequests: number,
): Promise<void> {
  return invoke("set_sync_settings", { provider, url, username, password, syncOnOpen, syncOnChange, syncIntervalMinutes, syncFolderRequests });
}

export async function testSyncConnection(): Promise<void> {
  return invoke("test_sync_connection");
}

export async function syncNow(): Promise<void> {
  return invoke("sync_now");
}

// ── Version History ──

export async function getNoteVersions(vaultPath: string, noteId: string): Promise<VersionEntry[]> {
  return invoke("get_note_versions", { vaultPath, noteId });
}

export async function getNoteVersionContent(
  vaultPath: string,
  noteId: string,
  timestamp: string,
): Promise<string> {
  return invoke("get_note_version_content", { vaultPath, noteId, timestamp });
}

export async function createVersion(
  vaultPath: string,
  path: string,
  noteId: string,
): Promise<void> {
  return invoke("create_version", { vaultPath, path, noteId });
}

// ── AI ──

export async function setAiSettings(
  provider: string | null,
  apiKey: string | null,
  model: string,
  writingStyle: string | null,
  baseUrl: string | null = null,
  ollamaApiKey: string | null = null,
  openaiCompatibleBaseUrl: string | null = null,
  openaiCompatibleApiKey: string | null = null,
): Promise<void> {
  return invoke("set_ai_settings", {
    provider, apiKey, model, writingStyle, baseUrl,
    ollamaApiKey, openaiCompatibleBaseUrl, openaiCompatibleApiKey,
  });
}

export async function testAiConnection(): Promise<void> {
  return invoke("test_ai_connection");
}

export async function aiAsk(
  action: string,
  text: string,
  customPrompt: string | null,
  requestId: string,
): Promise<void> {
  return invoke("ai_ask", { action, text, customPrompt, requestId });
}

export async function getInstallType(): Promise<string> {
  return invoke("get_install_type");
}

export async function isMobilePlatform(): Promise<boolean> {
  return invoke("is_mobile_platform");
}

export async function getPendingOpenFile(): Promise<PendingOpenFile | null> {
  return invoke("get_pending_open_file");
}

export async function ackPendingOpenFile(id: number): Promise<void> {
  return invoke("ack_pending_open_file", { id });
}
