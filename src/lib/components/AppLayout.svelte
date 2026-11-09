<script lang="ts">
	import { onMount, onDestroy, tick, untrack } from 'svelte';
	import { listen } from '@tauri-apps/api/event';
	import Sidebar from './Sidebar.svelte';
	import NoteList from './NoteList.svelte';
	import Editor from './Editor.svelte';
	import SearchPanel from './SearchPanel.svelte';
	import CommandPalette from './CommandPalette.svelte';
	import SettingsPanel from './SettingsPanel.svelte';
	import InfoPanel from './InfoPanel.svelte';
	import TitleBar from './TitleBar.svelte';
	import ResizeHandle from './ResizeHandle.svelte';
	import TagLabel from './TagLabel.svelte';
	import { getCurrentWindow } from '@tauri-apps/api/window';
	import {
		sidebarWidth,
		notelistWidth,
		outlineWidth,
		sidebarCollapsed,
		notelistCollapsed,
		collapsedNotebooks,
		showSearch,
		showCommandPalette,
		theme,
		resolvedTheme,
		customThemes,
		focusMode,
		readOnly,
		activeNote,
		activeNotePath,
		activeNotebook,
		editorDirty,
		showInfo,
		showSettings,
		sourceMode,
		mobileView,
		appConfig,
		quickAccessPaths,
		tags,
		notes,
		notebooks,
		rootNoteCount,
		viewMode,
		sortMode,
		groupNotesByDate,
		tasksLayout,
		tasksHideCompleted,
		tasksOnlyFlagged,
		tasksSort,
		activeTag,
		updateAvailable as globalUpdateAvailable,
		settingsTab,
		navHistory,
		viewerNote,
		notebookSortMode,
		notebookOrder,
		noteOrder,
		syncState,
		platformIsMobile,
		activeVaultConfig,
		activateVaultData,
	} from '$lib/stores/app';
	import { keybindings, matchAction } from '$lib/keybindings';

	const appWindow = getCurrentWindow();
	const isMac = navigator.platform.startsWith('Mac');
	const isMobile = $derived($platformIsMobile);
	import { loadVaultState, saveVaultState, readNote, deleteNote, createDailyNote, createBackup, getPendingOpenFile, ackPendingOpenFile, addQuickAccess, removeQuickAccess, getQuickAccess, setTheme, syncNow, setTaskDone, setTaskPriority, setTaskDue } from '$lib/api';
	import { darkThemes, isAndroid } from '$lib/platform';
	import { openNoteWindow } from '$lib/utils/window';
	import { normalizeStartupView, resolveStartupTarget } from '$lib/utils/startup-view';
	import { isCurrentVaultEvent, sameNoteTags, updateNoteList } from '$lib/utils/note-list-updates';
	import { beginNoteNavigation, isCurrentNoteNavigation, cancelNoteNavigation } from '$lib/utils/note-navigation';
	import { isPathInDirectory, isTrashedNote } from '$lib/utils/paths';
	import { get } from 'svelte/store';
	import type { VaultState, FileEvent, NotebookEntry, NoteEntry, TaskItem, SearchResult } from '$lib/types';
	import { completeWindowClose, cancelWindowClose } from '$lib/api';
	import type { StartupTarget } from '$lib/utils/startup-view';

	function findNotebookByPath(list: NotebookEntry[], relPath: string): NotebookEntry | null {
		for (const nb of list) {
			if (nb.relative_path === relPath) return nb;
			const found = findNotebookByPath(nb.children, relPath);
			if (found) return found;
		}
		return null;
	}

	let sidebar = $state<Sidebar>();
	let noteList = $state<NoteList>();
	let editor = $state<Editor>();
	let pendingSearchReveal = $state<{ path: string; token: number } | null>(null);
	let unlistenFileChange: (() => void) | null = null;
	let unlistenClose: (() => void) | null = null;
	let unlistenVaultRestored: (() => void) | null = null;
	let fileRefreshTimer: ReturnType<typeof setTimeout> | null = null;
	const ownerVault = get(appConfig)?.active_vault ?? null;
	activateVaultData(ownerVault);
	let stateReady = false;
	let restoringNote = false;
	let disposed = false;
	let navigationToken = 0;
	let persistTimer: ReturnType<typeof setTimeout> | null = null;
	let removePopstateListener: (() => void) | null = null;
	const backupListeners = new Set<() => void>();
	let backupInFlight = false;
	let pendingOpenDrain: Promise<void> | null = null;
	let unlistenPendingOpen: (() => void) | null = null;
	let pendingOpenNotification = false;
	let removeMobileFlushListeners: (() => void) | null = null;
	const recentChanges = new Map<string, NoteEntry>();
	let initialRefreshPending = true;
	let fileRefreshPending = false;
	let refreshLists = false;
	let fileRefreshRequest = 0;

	function ownerIsCurrent() {
		return !disposed && (get(appConfig)?.active_vault ?? null) === ownerVault;
	}

	function beginNavigation() {
		return navigationToken = beginNoteNavigation();
	}

	function navigationIsCurrent(token: number) {
		return ownerIsCurrent() && isCurrentNoteNavigation(token);
	}

	function scheduleFileRefresh(lists: boolean) {
		if (!ownerIsCurrent()) return;
		fileRefreshPending = true;
		refreshLists ||= lists;
		// Let the initial reads finish; note updates are replayed by NoteList.
		if (initialRefreshPending) return;
		const request = ++fileRefreshRequest;
		if (fileRefreshTimer) clearTimeout(fileRefreshTimer);
		fileRefreshTimer = setTimeout(async () => {
			fileRefreshTimer = null;
			if (!ownerIsCurrent() || request !== fileRefreshRequest) return;
			fileRefreshPending = false;
			const lists = refreshLists;
			refreshLists = false;
			if (lists) {
				await Promise.all([sidebar?.refresh(), noteList?.refresh(true)]);
				if (!ownerIsCurrent() || request !== fileRefreshRequest) return;
				if (isMobile) await sidebar?.refreshTags();
			} else {
				await sidebar?.refreshTags();
			}
		}, isMobile ? 1000 : 300);
	}

	function handleTagsChanged(path: string, nextTags: string[]) {
		if (!ownerIsCurrent()) return;
		// Record successful saves before patching lists so their delayed watcher
		// event does not schedule a second scan.
		const previous = recentChanges.get(path) ?? get(notes).find((note) => note.path === path);
		if (previous) {
			const updated = { ...previous, meta: { ...previous.meta, tags: nextTags } };
			recentChanges.set(path, updated);
			if (!sameNoteTags(previous, updated)) scheduleFileRefresh(false);
		} else {
			scheduleFileRefresh(false);
		}
	}

	async function applyStartupTarget(target: StartupTarget): Promise<boolean> {
		if (!ownerIsCurrent()) return false;
		if (target.mode === 'notebook') {
			const vault = $appConfig?.active_vault;
			const notebook = target.notebookPath === '' && vault
				? { name: 'Unfiled Notes', path: vault, relative_path: '', children: [], note_count: $rootNoteCount }
				: findNotebookByPath($notebooks, target.notebookPath);
			if (!notebook) return false;
			const changed = $viewMode !== 'notebook' || $activeNotebook?.relative_path !== notebook.relative_path;
			$viewMode = 'notebook';
			$activeNotebook = notebook;
			$activeTag = null;
			if (changed) await noteList?.refresh();
			return true;
		}

		if (target.mode === 'tag') {
			const changed = $viewMode !== 'tag' || $activeTag !== target.tag;
			$viewMode = 'tag';
			$activeTag = target.tag;
			$activeNotebook = null;
			if (changed) await noteList?.refresh();
			return true;
		}

		const changed = $viewMode !== target.mode || $activeNotebook !== null || $activeTag !== null;
		$viewMode = target.mode;
		$activeNotebook = null;
		$activeTag = null;
		if (changed) await noteList?.refresh();
		return true;
	}

	let unlistenOpenFile: (() => void) | null = null;

	// Mobile editor header helpers
	let noteRelativePath = $derived($activeNotePath && $appConfig?.active_vault ? $activeNotePath.replace($appConfig.active_vault + '/', '') : '');
	let isQuickAccess = $derived(noteRelativePath ? $quickAccessPaths.includes(noteRelativePath) : false);

	// Only the actions used while writing stay in the header row. Everything else
	// lives in the three-dot menu, so the row fits a phone at any width.
	const mobileHeaderIcons = {
		edit: '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7"/><path d="M18.5 2.5a2.121 2.121 0 013 3L12 15l-4 1 1-4 9.5-9.5z"/></svg>',
		view: '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/><circle cx="12" cy="12" r="3"/></svg>',
		find: '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/></svg>',
		pin: '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 17v5"/><path d="M9 2h6l-1 7h4l-2 4H8l-2-4h4L9 2z"/></svg>',
		quickAccess: '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" /></svg>',
		outline: '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="4" y1="6" x2="20" y2="6"/><line x1="8" y1="12" x2="20" y2="12"/><line x1="8" y1="18" x2="20" y2="18"/><circle cx="4" cy="12" r="1" fill="currentColor"/><circle cx="4" cy="18" r="1" fill="currentColor"/></svg>',
		history: '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/></svg>',
		info: '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><line x1="12" y1="16" x2="12" y2="12"/><line x1="12" y1="8" x2="12.01" y2="8"/></svg>',
		graph: '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="6" cy="6" r="3"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="18" r="3"/><line x1="8.5" y1="7.5" x2="15.5" y2="16.5"/><line x1="15.5" y1="7.5" x2="8.5" y2="16.5"/></svg>',
		ai: '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 8V4l-2-2"/><rect x="4" y="8" width="16" height="12" rx="2"/><path d="M2 14h2"/><path d="M20 14h2"/><path d="M9 13v2"/><path d="M15 13v2"/></svg>',
		sync: '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 2v6h-6"/><path d="M3 12a9 9 0 0115-6.7L21 8"/><path d="M3 22v-6h6"/><path d="M21 12a9 9 0 01-15 6.7L3 16"/></svg>',
		source: '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="16 18 22 12 16 6" /><polyline points="8 6 2 12 8 18" /></svg>',
	} as const;

	interface MobileHeaderAction {
		id: string;
		label: string;
		title: string;
		active?: boolean;
		disabled?: boolean;
		icon: string;
		run: () => void | Promise<void>;
	}

	let infoOpen = $state(false);
	let mobileActionMenu = $state(false);
	let mobileActionMenuEl = $state<HTMLDivElement | null>(null);
	let mobileActionMenuTrigger = $state<HTMLButtonElement | null>(null);
	const mobileActionMenuId = 'mobile-note-actions-menu';

	async function toggleQuickAccess() {
		if (!noteRelativePath || !ownerVault) return;
		try {
			if (isQuickAccess) {
				await removeQuickAccess(ownerVault, noteRelativePath);
			} else {
				await addQuickAccess(ownerVault, noteRelativePath);
			}
			if (!ownerIsCurrent()) return;
			const qaNotes = await getQuickAccess(ownerVault);
			if (ownerIsCurrent()) $quickAccessPaths = qaNotes.map(n => n.relative_path);
		} catch (e) {
			console.error('Quick access toggle failed:', e);
		}
	}

	let mobileEditorActions = $derived.by<MobileHeaderAction[]>(() => {
		if ($mobileView !== 'editor') return [];
		// Reading editorDirty refreshes the pin state after an in-place note edit.
		void $editorDirty;
		const actions: MobileHeaderAction[] = [
			{
				id: 'edit',
				label: $readOnly ? 'Switch to edit mode' : 'Switch to view mode',
				title: $readOnly ? 'Edit' : 'View',
				active: $readOnly,
				disabled: isTrashedNote($activeNotePath, ownerVault),
				icon: $readOnly ? mobileHeaderIcons.view : mobileHeaderIcons.edit,
				run: () => { $readOnly = !$readOnly; },
			},
			{
				id: 'find',
				label: 'Find in note',
				title: 'Find in note',
				icon: mobileHeaderIcons.find,
				run: () => editor?.openNoteSearch(),
			},
			{
				id: 'pin',
				label: $activeNote?.meta.pinned ? 'Unpin note' : 'Pin note',
				title: 'Pin',
				active: $activeNote?.meta.pinned,
				icon: mobileHeaderIcons.pin,
				run: () => {
					if (!$activeNote) return;
					$activeNote.meta.pinned = !$activeNote.meta.pinned;
					editor?.markEdited();
				},
			},
			{
				id: 'quick-access',
				label: isQuickAccess ? 'Remove from Quick Access' : 'Add to Quick Access',
				title: 'Quick Access',
				active: isQuickAccess,
				icon: mobileHeaderIcons.quickAccess,
				run: toggleQuickAccess,
			},
			{
				id: 'outline',
				label: 'Outline',
				title: 'Outline',
				icon: mobileHeaderIcons.outline,
				run: () => editor?.toggleOutlinePanel(),
			},
			{
				id: 'history',
				label: 'Version history',
				title: 'History',
				icon: mobileHeaderIcons.history,
				run: () => editor?.toggleHistoryPanel(),
			},
			{
				id: 'info',
				label: 'Note info',
				title: 'Note info',
				active: infoOpen,
				icon: mobileHeaderIcons.info,
				run: () => editor?.toggleInfoPanel(),
			},
		];
		if ($appConfig?.enable_wiki_links) {
			actions.push({
				id: 'graph',
				label: 'Graph view',
				title: 'Graph View',
				icon: mobileHeaderIcons.graph,
				run: () => editor?.toggleGraphView(),
			});
		}
		if ($appConfig?.ai_provider) {
			actions.push({
				id: 'ai',
				label: 'AI actions',
				title: 'AI Actions',
				icon: mobileHeaderIcons.ai,
				run: () => editor?.triggerAiMenu(),
			});
		}
		if (activeVaultConfig($appConfig)?.sync_provider === 'webdav') {
			actions.push({
				id: 'sync',
				label: 'Sync now',
				title: $syncState.error ? `Sync error: ${$syncState.error}` : ($syncState.running ? 'Syncing...' : 'Sync now'),
				active: $syncState.running,
				disabled: $syncState.running,
				icon: mobileHeaderIcons.sync,
				run: () => triggerSyncNow(),
			});
		}
		actions.push({
			id: 'source',
			label: $sourceMode ? 'Rich editor' : 'Source mode',
			title: $sourceMode ? 'Rich Editor' : 'Source Mode',
			active: $sourceMode,
			icon: mobileHeaderIcons.source,
			run: () => { $sourceMode = !$sourceMode; },
		});
		return actions;
	});

	// Everything the editor can do is in the menu; the row keeps only the
	// actions used while writing.
	const INLINE_MOBILE_ACTION_IDS = ['edit', 'find', 'info'];
	let inlineMobileEditorActions = $derived(
		mobileEditorActions.filter(action => INLINE_MOBILE_ACTION_IDS.includes(action.id))
	);

	async function toggleMobileActionMenu() {
		if (mobileActionMenu) {
			closeMobileActionMenu();
			return;
		}
		mobileActionMenu = true;
		await tick();
		mobileActionMenuEl?.querySelector<HTMLButtonElement>('.mobile-header-menu-item')?.focus({ preventScroll: true });
	}

	function closeMobileActionMenu(restoreFocus = true) {
		if (!mobileActionMenu) return;
		mobileActionMenu = false;
		if (restoreFocus) mobileActionMenuTrigger?.focus({ preventScroll: true });
	}

	function runMobileEditorAction(action: MobileHeaderAction) {
		closeMobileActionMenu(false);
		void action.run();
	}

	function onMobileActionMenuKeydown(event: KeyboardEvent) {
		if (event.key !== 'Escape') return;
		event.preventDefault();
		event.stopPropagation();
		closeMobileActionMenu();
	}

	function handleInfoChange(open: boolean) {
		infoOpen = open;
	}

	$effect(() => {
		// The menu belongs to one view; close it when that view changes.
		void $mobileView;
		void $activeNotePath;
		untrack(() => {
			if (mobileActionMenu) closeMobileActionMenu(false);
		});
	});
	let backupInterval: ReturnType<typeof setInterval> | null = null;
	let syncInterval: ReturnType<typeof setInterval> | null = null;
	let unlistenSync: Array<() => void> = [];
	let unsubDirty: (() => void) | null = null;
	let onChangeSyncTimer: ReturnType<typeof setTimeout> | null = null;
	let prevDirty = false;



	function parseFrequencyMs(freq: string): number {
		switch (freq) {
			case '6h': return 6 * 60 * 60 * 1000;
			case '12h': return 12 * 60 * 60 * 1000;
			case '7d': return 7 * 24 * 60 * 60 * 1000;
			case '24h': default: return 24 * 60 * 60 * 1000;
		}
	}

	async function checkScheduledBackup() {
		if (!ownerIsCurrent() || backupInFlight) return;
		const config = get(appConfig);
		const vault = activeVaultConfig(config);
		if (!config?.backup_enabled || !vault) return;
		const interval = parseFrequencyMs(config.backup_frequency);
		const last = vault.last_backup_time ? new Date(vault.last_backup_time).getTime() : 0;
		if (Date.now() - last < interval) return;
		backupInFlight = true;
		let unlisten: (() => void) | null = null;
		try {
			unlisten = await listen<{ vault_path: string; bookmark_id?: string | null; success: boolean; entry?: { created: string } }>('backup-done', (event) => {
				if (event.payload.vault_path !== ownerVault || (event.payload.bookmark_id ?? null) !== (vault.bookmark_id ?? null)) return;
				if (ownerIsCurrent() && event.payload.success && event.payload.entry?.created) {
					const current = get(appConfig);
					const active = activeVaultConfig(current);
					if (current && active) appConfig.set({
						...current,
						vaults: current.vaults.map((v) => v === active ? { ...v, last_backup_time: event.payload.entry!.created } : v)
					});
				}
				backupInFlight = false;
				unlisten?.();
				if (unlisten) backupListeners.delete(unlisten);
			});
			if (!ownerIsCurrent()) { unlisten(); backupInFlight = false; return; }
			backupListeners.add(unlisten);
			await createBackup(ownerVault!);
		} catch (error) {
			console.error('Failed to start scheduled backup:', error);
			unlisten?.();
			if (unlisten) backupListeners.delete(unlisten);
			backupInFlight = false;
		}
	}

	function syncConfigured(): boolean {
		return activeVaultConfig(get(appConfig))?.sync_provider === 'webdav';
	}

	// Auto-sync on a timer (only when configured and an interval is set).
	async function checkScheduledSync() {
		if (!ownerIsCurrent()) return;
		const vc = activeVaultConfig(get(appConfig));
		if (vc?.sync_provider !== 'webdav') return;
		const mins = vc.sync_interval_minutes ?? 0;
		if (!mins || get(syncState).running) return;
		const last = vc.last_sync_time ? new Date(vc.last_sync_time).getTime() : 0;
		if (Date.now() - last >= mins * 60 * 1000) {
			try { await syncNow(); } catch (_) {}
		}
	}

	export async function triggerSyncNow() {
		if (!ownerIsCurrent()) return;
		if (!syncConfigured() || get(syncState).running) return;
		try { await syncNow(); } catch (_) {}
	}

	// Track note navigation in history stack
	async function handlePathsMoved(oldPath: string, newPath: string) {
		if (!ownerIsCurrent()) return;
		navHistory.remap(oldPath, newPath);
		const path = $activeNotePath;
		if (!path || (path !== oldPath && !path.startsWith(oldPath + '/') && !path.startsWith(oldPath + '\\'))) return;
		const migrated = newPath + path.slice(oldPath.length);
		const previous = $activeNote;
		let content = previous ? { ...previous, path: migrated } : null;
		try {
			content = await readNote(ownerVault!, migrated);
		} catch (error) {
			console.error('Failed to reload relocated note:', error);
		}
		if (!ownerIsCurrent() || $activeNotePath !== path || $activeNote !== previous || !content) return;
		$activeNote = content;
		$activeNotePath = migrated;
		editor?.loadNote(migrated, content.content);
	}

	$effect(() => {
		const path = $activeNotePath;
		if (path) {
			navHistory.push(path);
			if (stateReady) queueMicrotask(drainPendingOpenFiles);
		}
	});

	async function navigateHistory(direction: -1 | 1) {
		const token = beginNavigation();
		if (!navigationIsCurrent(token)) return;
		if (editor && !(await editor.flushSave())) return;
		if (!navigationIsCurrent(token)) return;
		const path = navHistory.go(direction);
		if (!path) return;
		try {
			const content = await readNote(ownerVault!, path);
			if (!navigationIsCurrent(token)) return;
			if (editor && !(await editor.flushSave())) return;
			if (!navigationIsCurrent(token)) return;
			$activeNote = content;
			$activeNotePath = path;
			$editorDirty = false;
			editor?.loadNote(path, content.content);
		} catch (e) {
			console.error('Failed to navigate history:', e);
		}
	}

	async function handleOpenFile(filePath: string, token = beginNavigation()): Promise<boolean> {
		if (!navigationIsCurrent(token) || !filePath || !filePath.endsWith('.md')) return false;
		if (filePath === $activeNotePath) return true; // Never replace an open draft with a stale disk read.
		const isExternal = !isPathInDirectory(filePath, ownerVault);
		if (isExternal && isAndroid) return false;
		if (editor && !(await editor.flushSave())) return false;
		if (!navigationIsCurrent(token)) return false;
		try {
			const content = await readNote(ownerVault!, filePath);
			if (!navigationIsCurrent(token)) return false;
			$viewerNote = isExternal ? { path: filePath, content: content.content } : null;
			$activeNote = content;
			$activeNotePath = filePath;
			$editorDirty = false;
			if (isExternal) {
				$readOnly = true;
				$focusMode = true;
			}
			editor?.loadNote(filePath, content.content);
			return true;
		} catch (error) {
			console.error('Failed to open file:', error);
			return false;
		}
	}

	function drainPendingOpenFiles() {
		if (pendingOpenDrain) { pendingOpenNotification = true; return; }
		if (!stateReady || !ownerIsCurrent()) return;
		pendingOpenDrain = (async () => {
			try {
				while (ownerIsCurrent()) {
					const pending = await getPendingOpenFile();
					if (!pending || !ownerIsCurrent()) break;
					if (!(await handleOpenFile(pending.path))) break;
					if (!ownerIsCurrent()) break;
					await ackPendingOpenFile(pending.id);
				}
			} catch (error) {
				console.error('Failed to open pending file:', error);
			} finally {
				pendingOpenDrain = null;
				if (pendingOpenNotification) {
					pendingOpenNotification = false;
					queueMicrotask(drainPendingOpenFiles);
				}
			}
		})();
	}

	async function restoreLastNote(path: string, token: number) {
		const canRestore = () => navigationIsCurrent(token) && !$activeNotePath && !$editorDirty && !$viewerNote;
		if (!canRestore() || isTrashedNote(path, ownerVault)) return;
		const isExternal = !isPathInDirectory(path, ownerVault);
		if (isExternal && isAndroid) return;
		restoringNote = true;
		try {
			const content = await readNote(ownerVault!, path);
			if (!canRestore()) return;
			$activeNote = content;
			$activeNotePath = path;
			$editorDirty = false;
			if (isExternal) {
				$viewerNote = { path, content: content.content };
				$readOnly = true;
				$focusMode = true;
			}
			editor?.loadNote(path, content.content);
		} catch (_) {
		} finally {
			restoringNote = false;
			if (ownerIsCurrent()) persistState();
		}
	}

	async function persistVaultState() {
		if (!ownerIsCurrent() || !ownerVault || !stateReady || (restoringNote && !$activeNotePath)) return;
		const state: VaultState = {
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
			sort_mode: $sortMode,
			group_notes_by_date: $groupNotesByDate,
			last_view_mode: $viewMode,
			last_notebook: $activeNotebook?.relative_path ?? null,
			last_tag: $activeTag,
			tasks_layout: $tasksLayout,
			tasks_hide_completed: $tasksHideCompleted,
			tasks_only_flagged: $tasksOnlyFlagged,
			tasks_sort: $tasksSort
		};
		try {
			await saveVaultState(ownerVault, state);
		} catch (_) {}
	}

	function persistState() {
		if (!ownerIsCurrent() || !stateReady) return;
		if (persistTimer) clearTimeout(persistTimer);
		persistTimer = setTimeout(() => {
			persistTimer = null;
			void persistVaultState();
		}, 1000);
	}

	async function prepareVaultSwitch(): Promise<boolean> {
		const token = beginNavigation();
		if (editor && !(await editor.flushSave())) return false;
		if (!navigationIsCurrent(token)) return false;
		await persistVaultState();
		return navigationIsCurrent(token);
	}

	function handleSidebarResize(delta: number) {
		$sidebarWidth = Math.max(160, Math.min(400, $sidebarWidth + delta));
		persistState();
	}

	function handleNotelistResize(delta: number) {
		$notelistWidth = Math.max(200, Math.min(500, $notelistWidth + delta));
		persistState();
	}

	function revealNoteList() {
		$notelistCollapsed = false;
		tick().then(() => { if (ownerIsCurrent()) noteList?.refresh(); });
	}

	function toggleNoteList() {
		const nextCollapsed = !$notelistCollapsed;
		$notelistCollapsed = nextCollapsed;
		if (!nextCollapsed) {
			tick().then(() => { if (ownerIsCurrent()) noteList?.refresh(); });
		}
	}

	// Tasks view: the editor pane shows a placeholder until a task is opened from the list.
	let taskNoteOpened = $state(false);

	function handleNoteSelected(path: string, content: string, task?: TaskItem) {
		if (!ownerIsCurrent()) return;
		// Selecting a real vault note exits viewer mode
		$viewerNote = null;
		taskNoteOpened = true;
		editor?.loadNote(path, content, task);
		if (isMobile) $mobileView = 'editor';
	}

	async function selectNoteFromSwitcher(path: string, token = beginNavigation(), canSelect?: () => boolean): Promise<boolean> {
		const current = () => navigationIsCurrent(token) && (canSelect?.() ?? true);
		if (!current()) return false;
		const currentPath = $activeNotePath;
		if (currentPath && currentPath.replace(/\\/g, '/') === path.replace(/\\/g, '/')) return true;
		if (editor && !(await editor.flushSave())) return false;
		if (!current()) return false;
		try {
			const content = await readNote(ownerVault!, path);
			if (!current()) return false;
			if (editor && !(await editor.flushSave())) return false;
			if (!current()) return false;
			$viewerNote = null;
			$activeNote = content;
			$activeNotePath = path;
			$editorDirty = false;
			handleNoteSelected(path, content.content);
			return true;
		} catch (e) {
			console.error('Failed to switch note:', e);
			return false;
		}
	}

	async function selectNoteFromSearch(result: SearchResult, notebook: NotebookEntry | null, canSelect: () => boolean): Promise<boolean> {
		const { path } = result;
		const token = beginNavigation();
		if (!(await selectNoteFromSwitcher(path, token, canSelect)) || !navigationIsCurrent(token) || !canSelect()) return false;
		const mode = notebook ? 'notebook' : 'all';
		if ($viewMode !== mode || $activeNotebook?.path !== notebook?.path || $activeTag !== null) {
			$activeNotebook = notebook;
			$activeTag = null;
			$viewMode = mode;
			$notes = [];
		}
		$notelistCollapsed = false;
		pendingSearchReveal = { path, token };
		if (isMobile) $mobileView = 'editor';
		await tick();
		if (!navigationIsCurrent(token) || !canSelect()) return false;
		await editor?.revealSearchTarget(result.match_target, () => navigationIsCurrent(token) && canSelect());
		return navigationIsCurrent(token) && canSelect();
	}

	$effect(() => {
		const request = pendingSearchReveal;
		const list = noteList;
		if (!request || !list) return;
		// A hidden focus-mode list can mount later; only the originating navigation
		// may refresh and reveal it. Do not subscribe this effect to list internals.
		void untrack(async () => {
			const current = () => pendingSearchReveal === request && noteList === list
				&& navigationIsCurrent(request.token) && $activeNotePath === request.path;
			try {
				if (!current()) return;
				await list.refresh();
				if (!current()) return;
				const revealed = await list.revealNote(request.path, request.token);
				if (!revealed && current()) {
					// Search can find a newly indexed note before a cached list sees it.
					await list.refresh(true);
					if (current()) await list.revealNote(request.path, request.token);
				}
			} catch (error) {
				console.error('Failed to reveal search result:', error);
			} finally {
				if (pendingSearchReveal === request && (noteList === list
					|| !navigationIsCurrent(request.token) || $activeNotePath !== request.path)) pendingSearchReveal = null;
			}
		});
	});

	function handleViewChanged() {
		if (!ownerIsCurrent()) return;
		beginNavigation();
		taskNoteOpened = false;
		// Picking a notebook/tag/Tasks/etc. in the sidebar is a request to browse that view's
		// notes - so if the notes list was hidden, bring it back rather than navigating blindly
		// into a panel the user can't see. revealNoteList() re-mounts the list and refreshes it.
		if ($notelistCollapsed) {
			revealNoteList();
		} else {
			noteList?.refresh();
		}
		if (isMobile) $mobileView = 'notelist';
	}

	function createAndFocusNote() {
		return noteList?.handleCreateNote();
	}

	async function updateActiveNoteMetadata(patch: Pick<NoteEntry['meta'], 'pinned' | 'tags'>): Promise<boolean> {
		if (!ownerIsCurrent() || !$activeNote || !editor || editor.isMutationPending()) return false;
		$activeNote = { ...$activeNote, meta: { ...$activeNote.meta, ...patch } };
		editor.markEdited();
		return editor.forceSave();
	}

	// An active task mutation is serialized against both editing surfaces. The editor
	// briefly becomes read-only while the native command changes the disk note.
	async function changeTask(task: TaskItem, operation: () => Promise<void>) {
		if (!ownerIsCurrent()) return;
		try {
			if (task.note_path === $activeNotePath && editor) {
				await editor.applyNativeTaskChange(task.note_path, operation);
			} else {
				await operation();
			}
		} catch (error) {
			console.error('Failed to update task:', error);
		}
	}

	async function toggleTask(task: TaskItem) {
		await changeTask(task, () => setTaskDone(ownerVault!, task.note_path, task.line, task.raw_line, !task.completed));
	}

	async function changeTaskPriority(task: TaskItem, priority: string | null) {
		await changeTask(task, () => setTaskPriority(ownerVault!, task.note_path, task.line, task.raw_line, priority));
	}

	async function changeTaskDue(task: TaskItem, due: string | null) {
		await changeTask(task, () => setTaskDue(ownerVault!, task.note_path, task.line, task.raw_line, due));
	}

	async function handleDailyNote() {
		const token = beginNavigation();
		if (!navigationIsCurrent(token)) return;
		try {
			if (editor && !(await editor.flushSave())) return;
			if (!navigationIsCurrent(token)) return;
			const entry = await createDailyNote(ownerVault!);
			if (!navigationIsCurrent(token)) return;
			const content = await readNote(ownerVault!, entry.path);
			if (!navigationIsCurrent(token)) return;
			if (editor && !(await editor.flushSave())) return;
			if (!navigationIsCurrent(token)) return;
			$activeNote = content;
			$activeNotePath = entry.path;
			$editorDirty = false;
			editor?.loadNote(entry.path, content.content);
			$viewMode = 'daily';
			noteList?.refresh(true);
			sidebar?.refresh();
			if (isMobile) $mobileView = 'editor';
		} catch (e) {
			console.error('Failed to create/open daily note:', e);
		}
	}

	// Android back gesture / hardware back button support
	// We maintain a simple counter of how many views deep we are.
	// sidebar=0, notelist=1, editor=2. Each forward nav pushes, back pops.
	let historyDepth = 0;
	let navFromPopstate = false;

	if (get(platformIsMobile)) {
		history.replaceState({ mobileView: 'sidebar', depth: 0 }, '');

		$effect(() => {
			const view = $mobileView;
			if (navFromPopstate) {
				navFromPopstate = false;
				return;
			}
			const targetDepth = view === 'sidebar' ? 0 : view === 'notelist' ? 1 : 2;
			if (targetDepth > historyDepth) {
				// Forward navigation - push entries for each level skipped
				for (let d = historyDepth + 1; d <= targetDepth; d++) {
					const v = d === 1 ? 'notelist' : 'editor';
					history.pushState({ mobileView: v, depth: d }, '');
				}
				historyDepth = targetDepth;
			} else if (targetDepth < historyDepth) {
				const steps = historyDepth - targetDepth;
				historyDepth = targetDepth;
				navFromPopstate = true; // suppress the popstate that history.go triggers
				history.go(-steps);
			}
		});

		const onPopstate = (e: PopStateEvent) => {
			if (!ownerIsCurrent()) return;
			if (navFromPopstate) {
				navFromPopstate = false;
				return;
			}
			// If a modal is open, close it instead of navigating
			if ($showSettings || $showInfo || $showSearch || $showCommandPalette) {
				$showSettings = false;
				$showInfo = false;
				$showSearch = false;
				$showCommandPalette = false;
				// Re-push the current state so the next back still works
				history.pushState({ mobileView: $mobileView, depth: historyDepth }, '');
				return;
			}
			const state = e.state;
			const targetDepth = state?.depth ?? 0;
			historyDepth = targetDepth;
			navFromPopstate = true;
			if (targetDepth === 0) $mobileView = 'sidebar';
			else if (targetDepth === 1) $mobileView = 'notelist';
			else $mobileView = 'editor';
		};
		window.addEventListener('popstate', onPopstate);
		removePopstateListener = () => window.removeEventListener('popstate', onPopstate);
	}

	function mobileBack() {
		if ($mobileView === 'editor') $mobileView = 'notelist';
		else if ($mobileView === 'notelist') $mobileView = 'sidebar';
	}

	async function trashOpenNote(path: string): Promise<boolean> {
		if (path !== $activeNotePath || $viewerNote || $viewMode === 'trash') return false;
		try {
			await deleteNote(ownerVault!, path);
			if (!ownerIsCurrent()) return false;
			if (Object.hasOwn($noteOrder, path)) {
				const { [path]: _, ...rest } = $noteOrder;
				$noteOrder = rest;
			}
			$notes = $notes.filter((note) => note.path !== path);
			if ($activeNotePath === path) {
				$activeNote = null;
				$activeNotePath = null;
			}
			noteList?.refresh(true).catch((error) => console.error('Failed to refresh notes after trashing:', error));
			if (isMobile && !$activeNotePath) $mobileView = 'notelist';
			return true;
		} catch (error) {
			console.error('Failed to move open note to Trash:', error);
			return false;
		}
	}

	function handleMouseDown(e: MouseEvent) {
		if (e.button === 3) { e.preventDefault(); navigateHistory(-1); }
		if (e.button === 4) { e.preventDefault(); navigateHistory(1); }
	}

	function handleKeydown(e: KeyboardEvent) {
		const mod = e.ctrlKey || e.metaKey;
		const code = e.code;

		// Mod+K (insert link) is an editor formatting action and is not user-customizable.
		if (mod && code === 'KeyK' && !e.shiftKey && $activeNotePath && !$sourceMode) {
			e.preventDefault();
			editor?.addLinkFromToolbar();
		}

		// Ctrl/Cmd+Shift+Delete: move the open note to the trash.
		if (mod && e.shiftKey && code === 'Delete' && $activeNotePath) {
			e.preventDefault();
			editor?.moveOpenNoteToTrash();
			return;
		}

		const action = matchAction(e, $keybindings);
		if (action) {
			e.preventDefault();
			switch (action) {
				case 'nav-back':
					navigateHistory(-1);
					return;
				case 'nav-forward':
					navigateHistory(1);
					return;
				case 'new-note':
					createAndFocusNote();
					return;
				case 'toggle-theme': {
					const customTheme = $customThemes.find(theme => theme.id === $resolvedTheme);
					const isDark = darkThemes.includes($resolvedTheme) || (customTheme?.is_dark ?? false);
					const next = isDark ? 'light' : 'dark';
					$theme = next;
					setTheme(next);
					return;
				}
				case 'search-vault':
					$showSearch = true;
					return;
				case 'find-in-note':
					if ($activeNotePath) {
						editor?.openNoteSearch();
					} else {
						$showSearch = true;
					}
					return;
				case 'quick-open':
					$showCommandPalette = true;
					return;
				case 'save':
					editor?.forceSave();
					return;
				case 'toggle-source':
					$sourceMode = !$sourceMode;
					return;
				case 'open-new-window':
					if ($activeNotePath && $activeNote) {
						openNoteWindow(ownerVault!, $activeNotePath, $activeNote.meta.title);
					}
					return;
				case 'toggle-sidebar':
					$sidebarCollapsed = !$sidebarCollapsed;
					return;
				case 'toggle-notelist':
					toggleNoteList();
					return;
				case 'toggle-focus':
					$focusMode = !$focusMode;
					return;
				case 'toggle-readonly':
					if ($viewerNote) return; // viewer mode is always read-only
					$readOnly = !$readOnly;
					return;
				case 'fullscreen':
					appWindow.isFullscreen().then(fs => appWindow.setFullscreen(!fs));
					return;
			}
		}

		if (e.key === 'Escape') {
			const dismissesAppUi = $showSettings || $showInfo || $focusMode || $showSearch || $showCommandPalette;
			if (!dismissesAppUi) return;

			e.preventDefault();
			e.stopPropagation();
			if ($showSettings) $showSettings = false;
			else if ($showInfo) $showInfo = false;
			else if ($focusMode) $focusMode = false;
			else if ($showSearch) $showSearch = false;
			else if ($showCommandPalette) $showCommandPalette = false;
		}
	}

	$effect(() => {
		$collapsedNotebooks;
		persistState();
	});

	$effect(() => {
		$activeNotePath;
		persistState();
	});

	$effect(() => {
		$sidebarCollapsed;
		$notelistCollapsed;
		$outlineWidth;
		persistState();
	});

	$effect(() => {
		$notebookSortMode;
		$notebookOrder;
		persistState();
	});

	$effect(() => {
		$noteOrder;
		persistState();
	});

	$effect(() => {
		$sortMode;
		$groupNotesByDate;
		$tasksLayout;
		$tasksHideCompleted;
		$tasksOnlyFlagged;
		$tasksSort;
		persistState();
	});

	$effect(() => {
		$viewMode;
		$activeNotebook;
		$activeTag;
		persistState();
	});

	let activeRefreshRequest = 0;
	async function refreshActiveNote(path: string, restored = false) {
		if (path !== $activeNotePath || !$activeNote || !ownerIsCurrent()) return;
		const request = ++activeRefreshRequest;
		const openingNote = $activeNote;
		try {
			const content = await readNote(ownerVault!, path);
			if (!ownerIsCurrent() || request !== activeRefreshRequest || $activeNotePath !== path) return;
			if ($editorDirty) {
				if (restored && content.raw !== editor?.getAcceptedRaw()) editor?.flagExternalConflict();
				return;
			}
			if ($activeNote !== openingNote || content.raw === openingNote.raw) return;
			$activeNote = content;
			if ($viewerNote?.path === path) $viewerNote = { path, content: content.content };
			editor?.loadNote(path, content.content);
		} catch (error) {
			if (restored && ownerIsCurrent() && request === activeRefreshRequest && $activeNotePath === path) {
				if ($editorDirty) editor?.flagExternalConflict();
				else {
					$activeNotePath = null;
					$activeNote = null;
					$viewerNote = null;
				}
			} else if (!restored) console.error('Failed to refresh active note:', error);
		}
	}

	onMount(async () => {
		if (!ownerIsCurrent() || !ownerVault) return;
		const startupToken = beginNavigation();
		if (isMobile) {
			const flush = () => {
				if (ownerIsCurrent()) void editor?.flushSave();
			};
			const onVisibilityChange = () => {
				if (document.visibilityState === 'hidden') flush();
			};
			document.addEventListener('visibilitychange', onVisibilityChange);
			window.addEventListener('pagehide', flush);
			window.addEventListener('app-lifecycle-flush', flush);
			removeMobileFlushListeners = () => {
				document.removeEventListener('visibilitychange', onVisibilityChange);
				window.removeEventListener('pagehide', flush);
				window.removeEventListener('app-lifecycle-flush', flush);
			};
		}
		if (!isMobile) {
			unlistenClose = await appWindow.onCloseRequested(async (event) => {
				// Always prevent Tauri's implicit JS destroy(), which bypasses the tray policy.
				event.preventDefault();
				if (!ownerIsCurrent()) { await cancelWindowClose(); return; }
				try {
					if (editor && !(await editor.flushSave())) { await cancelWindowClose(); return; }
					if (!ownerIsCurrent()) { await cancelWindowClose(); return; }
					await persistVaultState();
					if (!ownerIsCurrent()) { await cancelWindowClose(); return; }
					await completeWindowClose();
				} catch (error) {
					console.error('Failed to close window:', error);
					try { await cancelWindowClose(); } catch (cancelError) { console.error('Failed to cancel window close:', cancelError); }
				}
			});
			if (!ownerIsCurrent()) { unlistenClose(); return; }
		}
		unlistenFileChange = await listen<FileEvent>('file-changed', ({ payload }) => {
			if (!ownerIsCurrent() || !isCurrentVaultEvent(payload, ownerVault)) return;
			if (!payload.note) {
				recentChanges.delete(payload.path);
				scheduleFileRefresh(true);
				return;
			}
			const note = payload.note;
			const visiblePrevious = get(notes).find((entry) => entry.path === note.path);
			const listPrevious = noteList?.updateNote(note) ?? visiblePrevious;
			const previous = recentChanges.get(note.path) ?? listPrevious;
			recentChanges.set(note.path, note);
			if (!noteList) {
				const key = $viewMode === 'notebook' && $activeNotebook ? `nb:${$activeNotebook.path}` :
					$viewMode === 'tag' && $activeTag ? `tag:${$activeTag}` : $viewMode;
				const updated = updateNoteList(get(notes), note, key, get(quickAccessPaths));
				if (updated !== get(notes)) notes.set(updated);
			}
			// Body/modified/preview changes do not change notebook counts or the tag index.
			if (!previous) scheduleFileRefresh(true);
			else if (!sameNoteTags(previous, note)) scheduleFileRefresh(false);
			if (payload.path === $activeNotePath) void refreshActiveNote(payload.path);
		});
		if (!ownerIsCurrent()) { unlistenFileChange(); return; }
		unlistenPendingOpen = await listen('open-file-pending', () => {
			drainPendingOpenFiles();
		});
		if (!ownerIsCurrent()) { unlistenPendingOpen(); return; }
		unlistenVaultRestored = await listen<{ vault_path: string }>('vault-restored', ({ payload }) => {
			if (!ownerIsCurrent() || payload.vault_path !== ownerVault) return;
			const path = $activeNotePath;
			if (path && isPathInDirectory(path, ownerVault)) void refreshActiveNote(path, true);
			scheduleFileRefresh(true);
		});
		if (!ownerIsCurrent()) { unlistenVaultRestored(); return; }
		unlistenOpenFile = await listen<string>('open-file', (event) => {
			if (ownerIsCurrent()) void handleOpenFile(event.payload);
		});
		if (!ownerIsCurrent()) { unlistenOpenFile(); return; }

		let lastNotePath: string | null = null;
		let lastViewMode = '';
		let lastNotebook: string | null = null;
		let lastTag: string | null = null;
		try {
			const state = await loadVaultState(ownerVault);
			if (!ownerIsCurrent()) return;
			$sidebarWidth = state.sidebar_width;
			$notelistWidth = state.notelist_width;
			if (typeof state.outline_width === 'number') $outlineWidth = state.outline_width;
			$sidebarCollapsed = state.sidebar_collapsed;
			$notelistCollapsed = state.notelist_collapsed ?? false;
			$collapsedNotebooks = state.collapsed_notebooks ?? [];
			$notebookSortMode = state.notebook_sort_mode === 'manual' ? 'manual' : 'alphabetical';
			$notebookOrder = state.notebook_order ?? {};
			$noteOrder = state.note_order ?? {};
			if (state.sort_mode === 'created' || state.sort_mode === 'title' || state.sort_mode === 'modified' || state.sort_mode === 'custom') $sortMode = state.sort_mode;
			if (typeof state.group_notes_by_date === 'boolean') $groupNotesByDate = state.group_notes_by_date;
			if (state.tasks_layout === 'calendar' || state.tasks_layout === 'list') $tasksLayout = state.tasks_layout;
			if (typeof state.tasks_hide_completed === 'boolean') $tasksHideCompleted = state.tasks_hide_completed;
			if (typeof state.tasks_only_flagged === 'boolean') $tasksOnlyFlagged = state.tasks_only_flagged;
			if (state.tasks_sort === 'due' || state.tasks_sort === 'priority' || state.tasks_sort === 'note') $tasksSort = state.tasks_sort;
			lastNotePath = state.last_open_note ?? null;
			lastViewMode = state.last_view_mode ?? '';
			lastNotebook = state.last_notebook ?? null;
			lastTag = state.last_tag ?? null;
		} catch (_) {}
		if (!ownerIsCurrent()) return;

		const restoreLastSession = $appConfig?.restore_last_session === true;

		// Display the remembered note independently of the metadata scans.
		if (restoreLastSession && lastNotePath) void restoreLastNote(lastNotePath, startupToken);

		await Promise.allSettled([sidebar?.refresh(), noteList?.refresh()]);
		if (!ownerIsCurrent()) return;

		// Only the all-notes list is a global tag source on mobile.
		if (isMobile) {
			if ($viewMode === 'all') {
				const tagMap = new Map<string, number>();
				for (const note of $notes) {
					for (const tag of note.meta.tags) {
						tagMap.set(tag, (tagMap.get(tag) ?? 0) + 1);
					}
				}
				$tags = Array.from(tagMap.entries()).sort((a, b) => a[0].localeCompare(b[0]));
			} else {
				await sidebar?.refreshTags();
				if (!ownerIsCurrent()) return;
			}
		}

		const startupTarget = resolveStartupTarget({
			startupView: $appConfig?.startup_view,
			restoreLastSession,
			lastViewMode,
			lastNotebook,
			lastTag
		});
		if (navigationIsCurrent(startupToken)) {
			if (!(await applyStartupTarget(startupTarget)) && navigationIsCurrent(startupToken)) {
				await applyStartupTarget({ mode: normalizeStartupView($appConfig?.startup_view) });
			}
		}
		if (!ownerIsCurrent()) return;

		initialRefreshPending = false;
		if (fileRefreshPending) scheduleFileRefresh(refreshLists);

		// Startup state is ready; open any queued OS request before optional sync.
		stateReady = true;
		drainPendingOpenFiles();

		// Scheduled backup: check on startup and every 5 minutes
		checkScheduledBackup();
		backupInterval = setInterval(checkScheduledBackup, 5 * 60 * 1000);

		// ── WebDAV sync: global status + auto-sync triggers ──
		unlistenSync.push(await listen<{ vault_path: string }>('sync-progress', ({ payload }) => {
			if (ownerIsCurrent() && payload.vault_path === ownerVault) syncState.set({ running: true, error: null });
		}));
		if (!ownerIsCurrent()) { unlistenSync.pop()?.(); return; }
		unlistenSync.push(await listen<{ vault_path: string; bookmark_id?: string | null; success: boolean; last_sync_time?: string | null }>('sync-done', ({ payload }) => {
			if (!ownerIsCurrent() || payload.vault_path !== ownerVault) return;
			syncState.set({ running: false, error: null });
			if (!payload.success || !payload.last_sync_time) return;
			const current = get(appConfig);
			const active = activeVaultConfig(current);
			if (current && active && (payload.bookmark_id ?? null) === (active.bookmark_id ?? null)) appConfig.set({
				...current,
				vaults: current.vaults.map((v) => v === active ? { ...v, last_sync_time: payload.last_sync_time } : v)
			});
		}));
		if (!ownerIsCurrent()) { unlistenSync.pop()?.(); return; }
		unlistenSync.push(await listen<{ vault_path: string; error?: string }>('sync-error', ({ payload }) => {
			if (ownerIsCurrent() && payload.vault_path === ownerVault) syncState.set({ running: false, error: payload.error ?? 'Sync failed' });
		}));
		if (!ownerIsCurrent()) { unlistenSync.pop()?.(); return; }

		// Sync when the vault opens (if enabled)
		if (syncConfigured() && activeVaultConfig(get(appConfig))?.sync_on_open) {
			try { await syncNow(); } catch (_) {}
		}
		if (!ownerIsCurrent()) return;

		// Auto-sync interval: check on startup and every minute
		checkScheduledSync();
		syncInterval = setInterval(checkScheduledSync, 60 * 1000);

		// Auto-sync on note change: a save flips editorDirty true -> false. Debounce a sync.
		unsubDirty = editorDirty.subscribe((d) => {
			if (!ownerIsCurrent()) return;
			const vc = activeVaultConfig(get(appConfig));
			if (prevDirty && !d && vc?.sync_provider === 'webdav' && vc?.sync_on_change) {
				if (onChangeSyncTimer) clearTimeout(onChangeSyncTimer);
				onChangeSyncTimer = setTimeout(() => { if (ownerIsCurrent() && !get(syncState).running) syncNow().catch(() => {}); }, 15000);
			}
			if (!d) drainPendingOpenFiles();
			prevDirty = d;
		});
		persistState();
	});

	onDestroy(() => {
		disposed = true;
		cancelNoteNavigation(navigationToken);
		if (persistTimer) clearTimeout(persistTimer);
		removePopstateListener?.();
		backupListeners.forEach((unlisten) => unlisten());
		backupListeners.clear();
		unlistenFileChange?.();
		unlistenVaultRestored?.();
		unlistenPendingOpen?.();
		unlistenClose?.();
		if (fileRefreshTimer) clearTimeout(fileRefreshTimer);
		unlistenOpenFile?.();
		removeMobileFlushListeners?.();
		if (backupInterval) clearInterval(backupInterval);
		if (syncInterval) clearInterval(syncInterval);
		if (onChangeSyncTimer) clearTimeout(onChangeSyncTimer);
		unsubDirty?.();
		unlistenSync.forEach((u) => u());
	});
</script>

<svelte:window onkeydown={handleKeydown} onmousedown={isMobile ? undefined : handleMouseDown} />

{#if isMobile}
	<!-- ═══ MOBILE LAYOUT ═══ -->
	<div class="mobile-shell">
		<!-- Mobile Header -->
		<div class="mobile-header">
			{#if $mobileView !== 'sidebar'}
				<button class="mobile-header-btn" onclick={mobileBack} aria-label="Go back">
					<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						<path d="M15 18l-6-6 6-6"/>
					</svg>
				</button>
			{:else}
				<div class="mobile-header-brand">
					<svg width="22" height="22" viewBox="0 0 48 48" fill="none">
						<rect width="48" height="48" rx="12" fill="var(--accent)" />
						<circle cx="16" cy="16" r="3.5" fill="white" opacity="0.9" />
						<circle cx="32" cy="16" r="3.5" fill="white" opacity="0.9" />
						<circle cx="16" cy="32" r="3.5" fill="white" opacity="0.9" />
						<circle cx="32" cy="32" r="3.5" fill="white" opacity="0.9" />
					</svg>
				</div>
			{/if}
			{#if $mobileView !== 'editor'}
				<span class="mobile-header-title">
					{#if $mobileView === 'sidebar'}
						HelixNotes
					{:else}
						{#if $viewMode === 'notebook'}{$activeNotebook?.name ?? 'Notebook'}{:else if $viewMode === 'tag' && $activeTag}<TagLabel name={$activeTag} size={16} />{:else if $viewMode === 'quickaccess'}Quick Access{:else if $viewMode === 'daily'}Daily Notes{:else if $viewMode === 'tasks'}Tasks{:else if $viewMode === 'trash'}Trash{:else}All Notes{/if}
					{/if}
				</span>
				{#if $globalUpdateAvailable && $mobileView === 'sidebar'}
					<button class="mobile-update-badge" onclick={() => { $settingsTab = 'updates'; $showSettings = true; }}>
						v{$globalUpdateAvailable.version}
					</button>
				{/if}
			{/if}
			<div class="mobile-header-actions">
				{#if $mobileView === 'editor'}
					{#each inlineMobileEditorActions as action (action.id)}
						<button
							class="mobile-header-btn"
							class:active={action.active}
							disabled={action.disabled}
							onclick={() => void action.run()}
							title={action.title}
							aria-label={action.label}
						>
							<span class="mobile-header-btn-icon" class:sync-spin={action.id === 'sync' && $syncState.running}>{@html action.icon}</span>
						</button>
					{/each}
					<button
						class="mobile-header-btn"
						class:active={mobileActionMenu}
						onclick={toggleMobileActionMenu}
						title="All note actions"
						aria-label="All note actions"
						aria-haspopup="menu"
						aria-expanded={mobileActionMenu}
						aria-controls={mobileActionMenu ? mobileActionMenuId : undefined}
						bind:this={mobileActionMenuTrigger}
					>
						<svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
							<circle cx="12" cy="5" r="1.7"/><circle cx="12" cy="12" r="1.7"/><circle cx="12" cy="19" r="1.7"/>
						</svg>
					</button>
				{:else}
					<button class="mobile-header-btn" onclick={() => ($showSearch = true)} title="Search">
						<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
							<circle cx="11" cy="11" r="8" />
							<line x1="21" y1="21" x2="16.65" y2="16.65" />
						</svg>
					</button>
						<button class="mobile-header-btn" onclick={handleDailyNote} title="Daily Note">
						<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
							<rect x="3" y="4" width="18" height="18" rx="2" ry="2" />
							<line x1="16" y1="2" x2="16" y2="6" />
							<line x1="8" y1="2" x2="8" y2="6" />
							<line x1="3" y1="10" x2="21" y2="10" />
						</svg>
					</button>
					<button class="mobile-header-btn" onclick={() => ($showInfo = true)} title="Info">
						<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
							<circle cx="12" cy="12" r="10" />
							<line x1="12" y1="16" x2="12" y2="12" />
							<line x1="12" y1="8" x2="12.01" y2="8" />
						</svg>
					</button>
					<button class="mobile-header-btn" onclick={() => ($showSettings = true)} title="Settings">
						<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
							<circle cx="12" cy="12" r="3" />
							<path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 01-2.83 2.83l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 012.83-2.83l.06.06A1.65 1.65 0 009 4.68a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z" />
						</svg>
					</button>
				{/if}
			</div>
			{#if mobileActionMenu}
				<button type="button" class="mobile-header-menu-backdrop" tabindex="-1" aria-label="Close menu" onclick={() => closeMobileActionMenu()}></button>
				<div
					class="mobile-header-menu"
					id={mobileActionMenuId}
					role="menu"
					tabindex="-1"
					aria-label="All note actions"
					bind:this={mobileActionMenuEl}
					onkeydown={onMobileActionMenuKeydown}
				>
					{#each mobileEditorActions as action (action.id)}
						<button
							type="button"
							class="mobile-header-menu-item"
							class:active={action.active}
							disabled={action.disabled}
							role="menuitem"
							onclick={() => runMobileEditorAction(action)}
						>
							<span class="mobile-header-menu-icon" class:sync-spin={action.id === 'sync' && $syncState.running}>{@html action.icon}</span>
							<span class="mobile-header-menu-label">{action.label}</span>
						</button>
					{/each}
				</div>
			{/if}
		</div>

		<!-- Mobile Content -->
		<div class="mobile-content">
			<div class="mobile-panel" class:active={$mobileView === 'sidebar'}>
				<Sidebar bind:this={sidebar} onViewChanged={handleViewChanged} onBeforeNoteChange={() => editor?.flushSave() ?? Promise.resolve(true)} onBeforePathMutation={() => editor?.beginPathMutation() ?? Promise.resolve(() => {})} onPathsMoved={handlePathsMoved} />
			</div>
			<div class="mobile-panel" class:active={$mobileView === 'notelist'}>
				<NoteList bind:this={noteList} onNoteSelected={handleNoteSelected} onBeforeNoteSwitch={() => editor?.flushSave() ?? Promise.resolve(true)} onBeforeNoteDuplicate={() => editor?.forceSave() ?? Promise.resolve(true)} onBeforePathMutation={() => editor?.beginPathMutation() ?? Promise.resolve(() => {})} onPathsMoved={handlePathsMoved} onUpdateActiveNote={updateActiveNoteMetadata} onTagsChanged={handleTagsChanged} onRefreshTags={() => { if (!isMobile) void sidebar?.refreshTags(); }} onNoteMoved={() => sidebar?.refresh()} onNoteCreated={(token) => { editor?.focusTitle(token); }} onToggleTask={toggleTask} onSetTaskPriority={changeTaskPriority} onSetTaskDue={changeTaskDue} />
			</div>
			<div class="mobile-panel" class:active={$mobileView === 'editor'}>
				<Editor bind:this={editor} onMoveToTrash={trashOpenNote} onInfoChange={handleInfoChange} />
			</div>
		</div>

	</div>
{:else}
	<!-- ═══ DESKTOP LAYOUT ═══ -->
	<div class="app-shell">
		{#if $focusMode}
			<!-- svelte-ignore a11y_no_static_element_interactions -->
			<div class="focus-topbar" class:macos={isMac} onmousedown={(e) => { if (!(e.target as HTMLElement).closest('button')) appWindow.startDragging(); }}>
				<span class="focus-title">{$activeNote?.meta.title || 'Untitled'}</span>
				<div class="focus-controls">
					<button class="focus-btn focus-active" onclick={() => ($focusMode = false)} title="Exit focus mode (Escape)">
						<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
							<path d="M8 3v3a2 2 0 01-2 2H3m18 0h-3a2 2 0 01-2-2V3m0 18v-3a2 2 0 012-2h3M3 16h3a2 2 0 012 2v3"/>
						</svg>
					</button>
					<button class="focus-btn" class:focus-active={$readOnly} onclick={() => ($readOnly = !$readOnly)} title={$readOnly ? 'Switch to Edit Mode' : 'Switch to View Mode'}>
						<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
							{#if $readOnly}
								<path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" />
								<circle cx="12" cy="12" r="3" />
							{:else}
								<path d="M17.94 17.94A10.07 10.07 0 0112 20c-7 0-11-8-11-8a18.45 18.45 0 015.06-5.94" />
								<path d="M9.9 4.24A9.12 9.12 0 0112 4c7 0 11 8 11 8a18.5 18.5 0 01-2.16 3.19" />
								<line x1="1" y1="1" x2="23" y2="23" />
							{/if}
						</svg>
					</button>
					{#if !isMac}
					<button class="focus-btn" onmousedown={(e) => e.stopPropagation()} onclick={() => appWindow.minimize()} title="Minimize">
						<svg width="10" height="10" viewBox="0 0 10 10"><line x1="1" y1="5" x2="9" y2="5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/></svg>
					</button>
					<button class="focus-btn" onmousedown={(e) => e.stopPropagation()} onclick={() => appWindow.toggleMaximize()} title="Maximize">
						<svg width="10" height="10" viewBox="0 0 10 10"><rect x="1" y="1" width="8" height="8" rx="1" fill="none" stroke="currentColor" stroke-width="1.2"/></svg>
					</button>
					<button class="focus-btn focus-close" onmousedown={(e) => e.stopPropagation()} onclick={() => appWindow.close()} title="Close">
						<svg width="10" height="10" viewBox="0 0 10 10"><line x1="1.5" y1="1.5" x2="8.5" y2="8.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/><line x1="8.5" y1="1.5" x2="1.5" y2="8.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/></svg>
					</button>
					{/if}
				</div>
			</div>
		{:else}
			<TitleBar onNewNote={createAndFocusNote} onDailyNote={handleDailyNote} onSelectNote={selectNoteFromSwitcher} onBeforeVaultSwitch={prepareVaultSwitch} />
		{/if}
		<div class="app-layout">
			{#if !$focusMode}
				<div class="sidebar-panel" style="width: {$sidebarCollapsed ? 44 : $sidebarWidth}px">
					<Sidebar bind:this={sidebar} onViewChanged={handleViewChanged} onBeforeNoteChange={() => editor?.flushSave() ?? Promise.resolve(true)} onBeforePathMutation={() => editor?.beginPathMutation() ?? Promise.resolve(() => {})} onPathsMoved={handlePathsMoved} />
				</div>

				{#if !$sidebarCollapsed}
					<ResizeHandle onResize={handleSidebarResize} />
				{/if}

				{#if !$notelistCollapsed}
					<div class="notelist-panel" style="width: {$notelistWidth}px">
						<NoteList bind:this={noteList} onNoteSelected={handleNoteSelected} onBeforeNoteSwitch={() => editor?.flushSave() ?? Promise.resolve(true)} onBeforeNoteDuplicate={() => editor?.forceSave() ?? Promise.resolve(true)} onBeforePathMutation={() => editor?.beginPathMutation() ?? Promise.resolve(() => {})} onPathsMoved={handlePathsMoved} onUpdateActiveNote={updateActiveNoteMetadata} onTagsChanged={handleTagsChanged} onRefreshTags={() => { if (!isMobile) void sidebar?.refreshTags(); }} onNoteMoved={() => sidebar?.refresh()} onNoteCreated={(token) => { editor?.focusTitle(token); }} onToggleTask={toggleTask} onSetTaskPriority={changeTaskPriority} onSetTaskDue={changeTaskDue} />
					</div>

					<ResizeHandle onResize={handleNotelistResize} />
				{:else}
					<div class="notelist-restore-panel">
						<button class="notelist-restore-btn" onclick={revealNoteList} title={`Show notes list (${isMac ? '⌘' : 'Ctrl'}+Shift+\\)`} aria-label="Show notes list">
							<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
								<line x1="4" y1="6" x2="16" y2="6" />
								<line x1="4" y1="12" x2="16" y2="12" />
								<line x1="4" y1="18" x2="16" y2="18" />
								<polyline points="19 8 21 12 19 16" />
							</svg>
						</button>
					</div>
				{/if}
			{/if}

			<div class="editor-panel">
				<Editor bind:this={editor} onMoveToTrash={trashOpenNote} onInfoChange={handleInfoChange} />
				{#if $viewMode === 'tasks' && !taskNoteOpened}
					<div class="tasks-editor-placeholder">
						<svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
							<path d="M9 11l3 3L22 4"/><path d="M21 12v7a2 2 0 01-2 2H5a2 2 0 01-2-2V5a2 2 0 012-2h11"/>
						</svg>
						<p>Select a task to open its note</p>
					</div>
				{/if}
			</div>
		</div>
	</div>
{/if}

<SearchPanel onSelectNote={selectNoteFromSearch} />
<CommandPalette onNavigate={handleViewChanged} />
<SettingsPanel onBeforeVaultSwitch={prepareVaultSwitch} />
<InfoPanel />

<style>
	/* ═══ DESKTOP STYLES ═══ */
	.app-shell {
		display: flex;
		flex-direction: column;
		height: 100vh;
		overflow: hidden;
	}

	.app-layout {
		display: flex;
		flex: 1;
		overflow: hidden;
	}

	.sidebar-panel {
		flex-shrink: 0;
		height: 100%;
		overflow: hidden;
	}

	.notelist-panel {
		flex-shrink: 0;
		height: 100%;
		overflow: hidden;
	}

	.notelist-restore-panel {
		flex-shrink: 0;
		width: 36px;
		height: 100%;
		padding-top: 8px;
		display: flex;
		justify-content: center;
		background: var(--bg-primary);
		border-right: 1px solid var(--border-color);
	}

	.notelist-restore-btn {
		width: 24px;
		height: 24px;
		display: flex;
		align-items: center;
		justify-content: center;
		padding: 0;
		border: none;
		border-radius: 4px;
		background: transparent;
		color: var(--text-tertiary);
		cursor: pointer;
	}

	.notelist-restore-btn:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.editor-panel {
		flex: 1;
		height: 100%;
		overflow: hidden;
		min-width: 300px;
		position: relative;
	}

	.tasks-editor-placeholder {
		position: absolute;
		inset: 0;
		z-index: 5;
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		gap: 12px;
		background: var(--bg-primary);
		color: var(--text-tertiary);
		font-size: 14px;
	}

	.focus-topbar {
		display: flex;
		align-items: center;
		height: 34px;
		background: var(--bg-secondary);
		border-bottom: 1px solid var(--border-light);
		user-select: none;
		flex-shrink: 0;
		padding: 0 8px 0 16px;
	}

	.focus-title {
		flex: 1;
		font-size: 12px;
		color: var(--text-tertiary);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		pointer-events: none;
	}

	.focus-controls {
		display: flex;
		align-items: center;
		gap: 2px;
	}

	.focus-btn {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 32px;
		height: 26px;
		border: none;
		background: none;
		color: var(--text-tertiary);
		cursor: pointer;
		border-radius: 4px;
		transition: background 0.1s, color 0.1s;
	}

	.focus-btn.focus-active {
		color: var(--text-accent);
		background: var(--accent-light);
	}

	.focus-btn:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.focus-close:hover {
		background: #e81123;
		color: white;
	}

	.focus-topbar.macos {
		padding-left: 78px;
	}

	/* ═══ MOBILE STYLES ═══ */
	.mobile-shell {
		display: flex;
		flex-direction: column;
		height: 100dvh;
		box-sizing: border-box;
		padding:
			env(safe-area-inset-top, 0px)
			env(safe-area-inset-right, 0px)
			env(safe-area-inset-bottom, 0px)
			env(safe-area-inset-left, 0px);
		overflow: hidden;
		background: var(--bg-primary);
	}

	.mobile-header {
		position: relative;
		z-index: 20;
		display: flex;
		align-items: center;
		height: 52px;
		padding: 0 8px;
		background: var(--bg-secondary);
		border-bottom: 1px solid var(--border-color);
		flex-shrink: 0;
		gap: 4px;
	}

	.mobile-header-brand {
		display: flex;
		align-items: center;
		padding: 0 8px;
	}

	.mobile-header-title {
		flex: 1;
		display: flex;
		align-items: center;
		min-width: 0;
		font-size: 17px;
		font-weight: 600;
		color: var(--text-primary);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		padding: 0 4px;
	}

	.mobile-update-badge {
		background: var(--accent);
		color: white;
		border: none;
		border-radius: 10px;
		padding: 2px 8px;
		font-size: 11px;
		font-weight: 600;
		font-family: inherit;
		cursor: pointer;
		white-space: nowrap;
		flex-shrink: 0;
	}

	.mobile-header-actions {
		display: flex;
		align-items: center;
		gap: 2px;
		margin-left: auto;
	}

	.mobile-header-btn {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 34px;
		height: 34px;
		border: none;
		background: none;
		color: var(--text-secondary);
		border-radius: 8px;
		cursor: pointer;
	}

	.mobile-header-btn svg {
		width: 16px;
		height: 16px;
	}

	.mobile-header-btn:active {
		background: var(--bg-hover);
	}

	.mobile-header-btn.active {
		color: var(--accent);
	}

	.mobile-header-btn-icon {
		display: flex;
		align-items: center;
		justify-content: center;
	}

	.mobile-header-menu-backdrop {
		position: fixed;
		inset: 0;
		z-index: 40;
		border: none;
		padding: 0;
		background: transparent;
	}

	.mobile-header-menu {
		position: absolute;
		top: calc(100% + 6px);
		right: 8px;
		z-index: 50;
		display: flex;
		flex-direction: column;
		min-width: 200px;
		max-width: calc(100vw - 16px);
		max-height: calc(100vh - 70px);
		overflow-y: auto;
		padding: 4px;
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 10px;
		box-shadow: var(--shadow-lg);
	}

	.mobile-header-menu-item {
		display: flex;
		align-items: center;
		gap: 10px;
		min-height: 44px;
		padding: 0 12px;
		border: none;
		border-radius: 8px;
		background: none;
		color: var(--text-primary);
		font-family: inherit;
		font-size: 14px;
		text-align: left;
		cursor: pointer;
	}

	.mobile-header-menu-item:hover:not(:disabled),
	.mobile-header-menu-item:active:not(:disabled) {
		background: var(--bg-hover);
	}

	.mobile-header-menu-item:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: -2px;
	}

	.mobile-header-menu-item.active {
		color: var(--accent);
	}

	.mobile-header-menu-item:disabled {
		opacity: 0.45;
		cursor: default;
	}

	.mobile-header-menu-icon {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 18px;
		height: 18px;
		flex-shrink: 0;
	}

	.mobile-header-menu-label {
		flex: 1;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.mobile-content {
		flex: 1;
		overflow: hidden;
		position: relative;
	}

	.mobile-panel {
		position: absolute;
		inset: 0;
		overflow: hidden;
		visibility: hidden;
		pointer-events: none;
		display: flex;
		flex-direction: column;
	}

	.mobile-panel.active {
		visibility: visible;
		pointer-events: auto;
	}

</style>
