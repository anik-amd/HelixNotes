import { writable, derived, get } from "svelte/store";
import { isMobile } from "$lib/platform";
import { compareNaturalNames } from "$lib/utils/natural-sort";
import type {
  AppConfig,
  CustomTheme,
  NoteEntry,
  NoteContent,
  NotebookEntry,
  VaultConfig,
  VaultState,
  ViewMode,
  SortMode,
  TagStyle,
} from "$lib/types";

// App state
export const appConfig = writable<AppConfig | null>(null);

// The active vault's config entry (where per-vault WebDAV sync settings live).
export function activeVaultConfig(c: AppConfig | null): VaultConfig | null {
  if (!c?.active_vault) return null;
  if (c.active_bookmark_id) {
    return c.vaults.find((v) => v.bookmark_id === c.active_bookmark_id) ?? null;
  }
  return c.vaults.find((v) => !v.bookmark_id && v.path === c.active_vault) ?? null;
}
export const vaultReady = writable(false);

// UI state
export const viewMode = writable<ViewMode>("all");
export const sortMode = writable<SortMode>("modified");
// Group the notes list under relative-date headers (All Notes, date sorts only). Persisted per vault.
export const groupNotesByDate = writable<boolean>(false);
export const tasksLayout = writable<"list" | "calendar">("list");
export const tasksHideCompleted = writable<boolean>(true);
export const tasksOnlyFlagged = writable<boolean>(false);
export const tasksSort = writable<"due" | "priority" | "note">("due");
export const sidebarCollapsed = writable(false);
export const notelistCollapsed = writable(false);
export const sidebarWidth = writable(220);
export const notelistWidth = writable(280);
export const outlineWidth = writable(220);
export const searchQuery = writable("");
export const showCommandPalette = writable(false);
export const showSearch = writable(false);
export const showSettings = writable(false);
export const settingsTab = writable<string | null>(null);
export const showInfo = writable(false);
export const notebookIcons = writable<Record<string, string>>({});
export const tagStyles = writable<Record<string, TagStyle>>({});
export const quickAccessPaths = writable<string[]>([]);
export const collapsedNotebooks = writable<string[]>([]);

// Notebook sort: 'alphabetical' (default) or 'manual' (drag-to-reorder, persisted in notebookOrder).
export const notebookSortMode = writable<"alphabetical" | "manual">(
  "alphabetical",
);
// Map from notebook absolute path → ordinal position (lower = earlier). Only consulted when notebookSortMode === 'manual'.
export const notebookOrder = writable<Record<string, number>>({});
export const noteOrder = writable<Record<string, number>>({});

// Data
export const notebooks = writable<NotebookEntry[]>([]);
export const rootNoteCount = writable<number>(0);
export const notes = writable<NoteEntry[]>([]);
export const tags = writable<[string, number][]>([]);
export const activeNote = writable<NoteContent | null>(null);
export const activeNotePath = writable<string | null>(null);
export const activeNotebook = writable<NotebookEntry | null>(null);
export const activeTag = writable<string | null>(null);

// External-file viewer mode: set when an .md file outside the active vault is opened.
// While set, the editor is forced read-only, autosave is suppressed, and a banner
// offering "Import to vault" / "Close" is shown.
export const viewerNote = writable<{ path: string; content: string } | null>(
  null,
);

// Mobile state
export const mobileView = writable<"sidebar" | "notelist" | "editor">(
  "sidebar",
);

// Editor state
export const editorDirty = writable(false);
export const sourceMode = writable(false);
export const focusMode = writable(false);
export const readOnly = writable(false);

// Theme
export const theme = writable<string>("system");
export const customThemes = derived(appConfig, ($c): CustomTheme[] => $c?.custom_themes ?? []);

// Whether the OS is currently in dark mode. Held in a store (rather than read at each call site)
// so the theme re-resolves when the user flips appearance while the app is open.
const darkQuery =
  typeof window !== "undefined" ? window.matchMedia("(prefers-color-scheme: dark)") : null;
export const systemPrefersDark = writable<boolean>(darkQuery?.matches ?? false);
darkQuery?.addEventListener("change", (e) => systemPrefersDark.set(e.matches));

// The theme to actually render: "system" resolves to the configured light/dark pair, everything
// else passes through. Falls back to the plain schemes when no pair is configured yet.
export const resolvedTheme = derived(
  [theme, appConfig, systemPrefersDark],
  ([$theme, $config, $prefersDark]): string => {
    if ($theme !== "system") return $theme;
    const fallback = $prefersDark ? "dark" : "light";
    const paired = $prefersDark ? $config?.system_dark_theme : $config?.system_light_theme;
    if (!paired) return fallback;
    if (paired.startsWith("custom-") && !$config?.custom_themes.some((item) => item.id === paired)) {
      return fallback;
    }
    return paired;
  },
);

// Sync (WebDAV) - global status so the top-bar button reflects any sync,
// whoever triggered it (manual button, settings, interval, on-change).
export interface SyncSummary {
  uploaded?: number;
  downloaded?: number;
  deleted_local?: number;
  deleted_remote?: number;
  conflicts?: number;
}
// `result` is what the last finished sync did, so Settings reports the same outcome for every
// way a sync can start.
export const syncState = writable<{
  running: boolean;
  error: string | null;
  result: { vault: string; summary: SyncSummary } | null;
}>({
  running: false,
  error: null,
  result: null,
});

// Update state
export const updateAvailable = writable<{
  version: string;
  body?: string;
} | null>(null);
export const updateObj = writable<any>(null);
export const installType = writable<string>("native");
// True only on the Android/iOS build. Reconfirmed from the backend at startup (see +layout). (#63)
export const platformIsMobile = writable<boolean>(isMobile);
export const androidApkUrl = writable<string | null>(null);

// Install types that handle their own updates (in-app auto-updater, or a
// package-manager notice for deb/aur). Anything else - e.g. a distro repo build
// that sets the HELIXNOTES_INSTALL_TYPE build flag (Solus, etc.) - is "managed":
// the app does no update check and shows no update UI at all.
const SELF_UPDATING_INSTALL_TYPES = ["macos", "windows", "deb", "aur", "appimage", "native", "android"];
export function isManagedInstall(type: string): boolean {
  return !SELF_UPDATING_INSTALL_TYPES.includes(type);
}

export async function checkForUpdate() {
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    const update = await check();
    if (update) {
      updateAvailable.set({ version: update.version, body: update.body });
      updateObj.set(update);
    }
  } catch {
    // Silent fail - don't disrupt app startup
  }
}

function isNewerVersion(remote: string, local: string): boolean {
  const r = remote.split(".").map(Number);
  const l = local.split(".").map(Number);
  for (let i = 0; i < Math.max(r.length, l.length); i++) {
    const rv = r[i] || 0;
    const lv = l[i] || 0;
    if (rv > lv) return true;
    if (rv < lv) return false;
  }
  return false;
}

export async function checkForUpdateMobile() {
  try {
    const { getVersion } = await import("@tauri-apps/api/app");
    const currentVersion = await getVersion();
    const res = await fetch("https://helixnotes.com/latest.json");
    if (!res.ok) return;
    const data = await res.json();
    if (data.version && isNewerVersion(data.version, currentVersion)) {
      updateAvailable.set({ version: data.version, body: data.notes });
      const android = data.platforms?.["android-universal"];
      if (android?.url) {
        androidApkUrl.set(android.url);
      }
    }
  } catch {
    // Silent fail
  }
}

// Note navigation history
interface NavHistoryState { stack: string[]; index: number; skipping: boolean; }
function createNavHistory() {
  const store = writable<NavHistoryState>({ stack: [], index: -1, skipping: false });
  return {
    subscribe: store.subscribe,
    clear() {
      store.set({ stack: [], index: -1, skipping: false });
    },
    push(path: string) {
      store.update(s => {
        if (s.skipping) return { ...s, skipping: false };
        if (s.stack[s.index] === path) return s;
        const trimmed = s.stack.slice(0, s.index + 1);
        return { stack: [...trimmed, path], index: trimmed.length, skipping: false };
      });
    },
    remap(oldPrefix: string, newPrefix: string) {
      const oldRoot = oldPrefix.replace(/\\/g, '/').replace(/\/+$/, '');
      const newRoot = newPrefix.replace(/[\\/]+$/, '');
      const insensitive = /^[A-Za-z]:\//.test(oldRoot) || oldRoot.startsWith('//');
      const matchRoot = insensitive ? oldRoot.toLowerCase() : oldRoot;
      store.update(s => ({
        ...s,
        stack: s.stack.map(path => {
          const normalized = path.replace(/\\/g, '/');
          const candidate = insensitive ? normalized.toLowerCase() : normalized;
          return candidate === matchRoot || candidate.startsWith(`${matchRoot}/`)
            ? newRoot + path.slice(oldRoot.length)
            : path;
        }),
      }));
    },
    go(direction: -1 | 1): string | null {
      let target: string | null = null;
      store.update(s => {
        const newIdx = s.index + direction;
        if (newIdx < 0 || newIdx >= s.stack.length) return s;
        target = s.stack[newIdx];
        return { ...s, index: newIdx, skipping: true };
      });
      return target;
    },
  };
}
export const navHistory = createNavHistory();
export const canGoBack = derived(navHistory, $h => $h.index > 0);
export const canGoForward = derived(navHistory, $h => $h.index < $h.stack.length - 1);

let dataVaultPath: string | null = null;

/** Never expose the previous vault's editable note while a new vault loads. */
export function activateVaultData(vaultPath: string | null): void {
  if (dataVaultPath === vaultPath) return;
  dataVaultPath = vaultPath;
  activeNote.set(null);
  activeNotePath.set(null);
  activeNotebook.set(null);
  activeTag.set(null);
  viewerNote.set(null);
  editorDirty.set(false);
  notes.set([]);
  notebooks.set([]);
  tags.set([]);
  rootNoteCount.set(0);
  quickAccessPaths.set([]);
  notebookIcons.set({});
  tagStyles.set({});
  collapsedNotebooks.set([]);
  notebookOrder.set({});
  noteOrder.set({});
  viewMode.set("all");
  sortMode.set("modified");
  groupNotesByDate.set(false);
  navHistory.clear();
}

// Derived
// Entries are replaced, not mutated, when a note changes, so a parsed date stays valid for its entry.
const noteTimestamps = { created: new WeakMap<NoteEntry, number>(), modified: new WeakMap<NoteEntry, number>() };

export const sortedNotes = derived(
  [notes, sortMode, viewMode, noteOrder],
  ([$notes, $sortMode, $viewMode, $noteOrder]) => {
    // Quick Access preserves stored order
    if ($viewMode === "quickaccess") return $notes;
    const customSortable =
      $viewMode === "all" || $viewMode === "notebook" || $viewMode === "tag";

    const pinned = $notes.filter((n) => n.meta.pinned);
    const unpinned = $notes.filter((n) => !n.meta.pinned);
    const dateField = $sortMode === "title" || ($sortMode === "custom" && customSortable)
      ? null
      : $sortMode === "created" ? "created" : "modified";
    const timestamps = dateField ? noteTimestamps[dateField] : null;
    if (dateField && timestamps) {
      for (const note of $notes) {
        if (!timestamps.has(note)) timestamps.set(note, Date.parse(note.meta[dateField]));
      }
    }

    const sortFn = (a: NoteEntry, b: NoteEntry) => {
      if (timestamps) return timestamps.get(b)! - timestamps.get(a)!;
      if ($sortMode === "custom") {
        const oa = $noteOrder[a.path] ?? Number.MAX_SAFE_INTEGER;
        const ob = $noteOrder[b.path] ?? Number.MAX_SAFE_INTEGER;
        if (oa !== ob) return oa - ob;
        return a.meta.title.localeCompare(b.meta.title);
      }
      return compareNaturalNames(a.meta.title, b.meta.title);
    };

    return [...pinned.sort(sortFn), ...unpinned.sort(sortFn)];
  },
);

export const vaultState = derived(
  [
    activeNotePath,
    sidebarWidth,
    notelistWidth,
    outlineWidth,
    sidebarCollapsed,
    notelistCollapsed,
    collapsedNotebooks,
    notebookSortMode,
    notebookOrder,
    noteOrder,
  ],
  ([
    $activeNotePath,
    $sidebarWidth,
    $notelistWidth,
    $outlineWidth,
    $sidebarCollapsed,
    $notelistCollapsed,
    $collapsedNotebooks,
    $notebookSortMode,
    $notebookOrder,
    $noteOrder,
  ]) => {
    return {
      last_open_note: $activeNotePath,
      sidebar_width: $sidebarWidth,
      notelist_width: $notelistWidth,
      outline_width: $outlineWidth,
      sidebar_collapsed: $sidebarCollapsed,
      notelist_collapsed: $notelistCollapsed,
      collapsed_notebooks: $collapsedNotebooks,
      notebook_sort_mode: $notebookSortMode,
      notebook_order: $notebookOrder,
      note_order: $noteOrder,
    } satisfies VaultState;
  },
);
