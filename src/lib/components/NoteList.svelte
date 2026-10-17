<script lang="ts">
	import { onDestroy, tick, untrack } from 'svelte';
	import { get } from 'svelte/store';
	import {
		notes,
		sortedNotes,
		activeNotePath,
		activeNote,
		viewMode,
		activeNotebook,
		activeTag,
		sortMode,
		groupNotesByDate,
		editorDirty,
		quickAccessPaths,
		appConfig,
		notelistCollapsed,
		noteOrder,
		notebooks,
		mobileView
	} from '$lib/stores/app';
	import {
		getNotes,
		readNote,
		createNote,
		duplicateNote,
		deleteNote,
		renameNote,
		saveNote,
		getTrash,
		restoreNote,
		restoreNotebook,
		permanentDelete,
		getQuickAccess,
		addQuickAccess,
		removeQuickAccess,
		reorderQuickAccess,
		moveNote,
		createDailyNote,
		revealFile
	} from '$lib/api';
	import { formatRelativeTime, formatDate } from '$lib/utils/time';
	import { openNoteWindow } from '$lib/utils/window';
	import { encodeNoteDragPaths } from '$lib/utils/note-drag';
	import { tagIterationKey } from '$lib/utils/tag-styles';
	import { updateNoteList } from '$lib/utils/note-list-updates';
	import { buildNoteListRows, noteListOffsets, noteListWindow, noteListRevealScrollTop } from '$lib/utils/note-list-window';
	import { beginNoteNavigation, isCurrentNoteNavigation, cancelNoteNavigation } from '$lib/utils/note-navigation';
	import type { NoteEntry, NoteMeta, TrashNotebookEntry, SortMode, TaskItem } from '$lib/types';
	import TasksView from './TasksView.svelte';
	import TagSuggestInput from './TagSuggestInput.svelte';
	import TagLabel from './TagLabel.svelte';
	import { isMobile, isAndroid } from '$lib/platform';

	let { onNoteSelected = (_path: string, _content: string, _task?: TaskItem) => {}, onNoteMoved = () => {}, onBeforeNoteSwitch = async () => true, onBeforeNoteDuplicate = async () => true, onBeforePathMutation = async () => null, onPathsMoved = async (_old: string, _next: string) => {}, onUpdateActiveNote, onTagsChanged, onRefreshTags, onNoteCreated = () => {}, onToggleTask = async (_t: TaskItem) => {}, onSetTaskPriority = async (_t: TaskItem, _p: string | null) => {}, onSetTaskDue = async (_t: TaskItem, _d: string | null) => {} }: {
		onNoteSelected?: (path: string, content: string, task?: TaskItem) => void;
		onNoteMoved?: () => void;
		onBeforeNoteSwitch?: () => Promise<boolean>;
		onBeforeNoteDuplicate?: () => Promise<boolean>;
		onBeforePathMutation?: () => Promise<(() => void) | null>;
		onPathsMoved?: (oldPath: string, newPath: string) => Promise<void>;
		onUpdateActiveNote: (patch: Pick<NoteMeta, 'pinned' | 'tags'>) => Promise<boolean>;
		onTagsChanged: (path: string, nextTags: string[]) => void;
		onRefreshTags: () => void;
		onNoteCreated?: (token: number) => void;
		onToggleTask?: (t: TaskItem) => Promise<void>;
		onSetTaskPriority?: (t: TaskItem, p: string | null) => Promise<void>;
		onSetTaskDue?: (t: TaskItem, d: string | null) => Promise<void>;
	} = $props();

	const modKey = navigator.platform.startsWith('Mac') ? '⌘' : 'Ctrl';
	let multiSelectMode = $state(false);
	let trashNotebooks = $state<TrashNotebookEntry[]>([]);
	let trashBusy = $state<string | null>(null);

	// Calendar state for daily notes view
	let calMonth = $state(new Date().getMonth());
	let calYear = $state(new Date().getFullYear());
	let dailyDates = $state<Set<string>>(new Set());

	const calMonthNames = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];
	// 0 = Sunday, 1 = Monday (default). Day names + grid offset follow the setting.
	const weekStartsOn = $derived($appConfig?.week_start === 'sunday' ? 0 : 1);
	const DOW = ['Su', 'Mo', 'Tu', 'We', 'Th', 'Fr', 'Sa'];
	const calDayNames = $derived([...DOW.slice(weekStartsOn), ...DOW.slice(0, weekStartsOn)]);

	function calDays(): Array<{ day: number; date: string; current: boolean }> {
		const first = new Date(calYear, calMonth, 1);
		const lastDay = new Date(calYear, calMonth + 1, 0).getDate();
		const startDow = (first.getDay() - weekStartsOn + 7) % 7;
		const cells: Array<{ day: number; date: string; current: boolean }> = [];
		for (let i = 0; i < startDow; i++) cells.push({ day: 0, date: '', current: false });
		for (let d = 1; d <= lastDay; d++) {
			const mm = String(calMonth + 1).padStart(2, '0');
			const dd = String(d).padStart(2, '0');
			const date = `${calYear}-${mm}-${dd}`;
			const now = new Date();
			const current = d === now.getDate() && calMonth === now.getMonth() && calYear === now.getFullYear();
			cells.push({ day: d, date, current });
		}
		return cells;
	}

	function calPrev() {
		if (calMonth === 0) { calMonth = 11; calYear--; }
		else calMonth--;
	}
	function calNext() {
		if (calMonth === 11) { calMonth = 0; calYear++; }
		else calMonth++;
	}
	function calToday() {
		const now = new Date();
		calMonth = now.getMonth();
		calYear = now.getFullYear();
	}

	async function handleDayClick(date: string, token = beginNavigation()) {
		if (!navigationIsCurrent(token)) return;
		try {
			const entry = await createDailyNote(ownerVault!, date);
			if (!navigationIsCurrent(token)) return;
			const content = await readNote(ownerVault!, entry.path);
			if (!navigationIsCurrent(token)) return;
			if (!(await onBeforeNoteSwitch())) return;
			if (!navigationIsCurrent(token)) return;
			$activeNote = content;
			$activeNotePath = entry.path;
			$editorDirty = false;
			onNoteSelected(entry.path, content.content);
			if (!dailyDates.has(date)) {
				dailyDates = new Set([...dailyDates, date]);
				onNoteMoved();
			}
			noteCache.clear();
			await refresh();
		} catch (e) {
			console.error('Failed to open daily note:', e);
		}
	}

	let compact = $derived($appConfig?.compact_notes ?? false);
	let showDates = $derived($appConfig?.show_note_dates !== false);
	let contextMenu = $state<{ x: number; y: number; note: NoteEntry } | null>(null);
	let sortMenu = $state<{ x: number; y: number } | null>(null);
	let editingNote = $state<string | null>(null);
	let editValue = $state('');
	let renameInput: HTMLInputElement | null = $state(null);
	let movePickerNote = $state<NoteEntry | null>(null);
	let tagEditNote = $state<NoteEntry | null>(null);
	let tagEditTags = $state<string[]>([]);

	// Multi-select
	let selectedPaths = $state<Set<string>>(new Set());
	let lastClickedPath = $state<string | null>(null);
	let batchMovePicker = $state(false);
	let batchTagEdit = $state(false);

	// Mobile long-press selection
	let longPressTimer: ReturnType<typeof setTimeout> | null = null;
	let longPressTriggered = false;
	let touchStartPos = { x: 0, y: 0 };
	const LONG_PRESS_MS = 500;
	const LONG_PRESS_MOVE_THRESHOLD = 10;

	function handleTouchStart(e: TouchEvent, note: NoteEntry) {
		longPressTriggered = false;
		const touch = e.touches[0];
		touchStartPos = { x: touch.clientX, y: touch.clientY };
		longPressTimer = setTimeout(() => {
			longPressTriggered = true;
			// Vibrate for haptic feedback if available
			if (navigator.vibrate) navigator.vibrate(30);
			const next = new Set(selectedPaths);
			if (next.size === 0 && $activeNotePath && $activeNotePath !== note.path) {
				next.add($activeNotePath);
			}
			if (next.has(note.path)) {
				next.delete(note.path);
			} else {
				next.add(note.path);
			}
			selectedPaths = next;
		}, LONG_PRESS_MS);
	}

	function handleTouchMove(e: TouchEvent) {
		if (!longPressTimer) return;
		const touch = e.touches[0];
		const dx = touch.clientX - touchStartPos.x;
		const dy = touch.clientY - touchStartPos.y;
		if (Math.abs(dx) > LONG_PRESS_MOVE_THRESHOLD || Math.abs(dy) > LONG_PRESS_MOVE_THRESHOLD) {
			clearTimeout(longPressTimer);
			longPressTimer = null;
		}
	}

	function handleTouchEnd() {
		if (longPressTimer) {
			clearTimeout(longPressTimer);
			longPressTimer = null;
		}
	}

	// Quick Access drag-to-reorder
	let qaDragFrom = $state<number | null>(null);
	let qaDragOver = $state<number | null>(null);
	let qaDragHalf = $state<'top' | 'bottom'>('bottom');
	let noteDragFrom = $state<number | null>(null);
	let noteDragOver = $state<number | null>(null);
	let noteDragHalf = $state<'top' | 'bottom'>('bottom');
	const customOrderViews = new Set(['all', 'notebook', 'tag']);
	let canCustomReorder = $derived($sortMode === 'custom' && customOrderViews.has($viewMode));

	let groupingActive = $derived($groupNotesByDate && $viewMode === 'all' && ($sortMode === 'created' || $sortMode === 'modified'));
	let listRows = $derived(buildNoteListRows(
		$sortedNotes,
		groupingActive ? ($sortMode === 'created' ? 'created' : 'modified') : null,
	));

	let listContainer = $state<HTMLDivElement>(null!);
	let virtualList = $state<HTMLDivElement>(null!);
	let scrollTop = $state(0);
	let stickToBottom = false;
	let listOrigin = $state(0);
	let containerHeight = $state(600);
	const rowHeights = new Map<string, number>();
	let heightRevision = $state(0);
	let rowObserver: ResizeObserver | null = null;
	const pendingHeights = new Map<string, number>();
	let measurementFrame: number | null = null;
	let disposed = false;
	const ownerVault = get(appConfig)?.active_vault ?? null;
	let navigationToken = 0;

	function ownerIsCurrent() {
		return !disposed && (get(appConfig)?.active_vault ?? null) === ownerVault;
	}

	function beginNavigation() {
		return navigationToken = beginNoteNavigation();
	}

	function navigationIsCurrent(token: number) {
		return ownerIsCurrent() && isCurrentNoteNavigation(token);
	}
	const BUFFER = 10;
	let offsets = $derived.by(() => {
		void heightRevision;
		return noteListOffsets(listRows, rowHeights, compact ? (isMobile ? 50 : 33) : (isMobile ? 90 : 62), 32);
	});
	let windowRows = $derived(noteListWindow(offsets, scrollTop - listOrigin, containerHeight, BUFFER));
	let visibleRows = $derived(listRows.slice(windowRows.start, windowRows.end));

	function applyMeasuredRows() {
		measurementFrame = null;
		if (disposed) return;
		if (!listContainer || listContainer.clientHeight === 0) {
			pendingHeights.clear();
			return;
		}
		if (virtualList) listOrigin = virtualList.offsetTop;
		const rows = listRows;
		const anchor = windowRows.anchor;
		const previousOffset = offsets[anchor];
		const previousScroll = scrollTop;
		const wasAtBottom = stickToBottom;
		let changed = false;
		for (const [key, height] of pendingHeights) {
			if (Math.abs((rowHeights.get(key) ?? 0) - height) > 0.5) {
				rowHeights.set(key, height);
				changed = true;
			}
		}
		pendingHeights.clear();
		if (!changed) return;
		heightRevision++;
		// Keep the visible row stationary as overscan rows acquire their actual heights.
		void tick().then(() => {
			if (disposed || listRows !== rows || !listContainer) return;
			if (wasAtBottom && stickToBottom) {
				listContainer.scrollTop = listContainer.scrollHeight;
			} else {
				if (listContainer.scrollTop !== previousScroll) return;
				listContainer.scrollTop += offsets[anchor] - previousOffset;
			}
			scrollTop = listContainer.scrollTop;
		});
	}

	export async function revealNote(path: string, token: number): Promise<boolean> {
		const view = cacheKey();
		const current = () => ownerIsCurrent() && isCurrentNoteNavigation(token)
			&& $activeNotePath === path && cacheKey() === view;
		if (!current()) return false;
		clearSelection();
		stickToBottom = false;
		// First render the estimated window, then refine using its actual row heights.
		for (let pass = 0; pass < 2; pass++) {
			await tick();
			if (!current() || !listContainer || !virtualList) return false;
			const index = listRows.findIndex(row => row.kind === 'note' && row.note.path === path);
			if (index < 0) return false;
			listOrigin = virtualList.offsetTop;
			listContainer.scrollTop = noteListRevealScrollTop(
				offsets, index, listContainer.scrollTop, listContainer.clientHeight, listOrigin,
			);
			scrollTop = listContainer.scrollTop;
			await tick();
			if (!current() || !virtualList) return false;
			for (const row of virtualList.querySelectorAll<HTMLDivElement>('.virtual-note-row')) {
				if (row.offsetHeight > 0) pendingHeights.set(row.dataset.rowKey!, row.offsetHeight);
			}
			if (measurementFrame !== null) cancelAnimationFrame(measurementFrame);
			applyMeasuredRows();
		}
		return current();
	}

	function measureRow(node: HTMLDivElement) {
		rowObserver ??= new ResizeObserver((entries) => {
			for (const entry of entries) {
				const key = (entry.target as HTMLElement).dataset.rowKey!;
				const height = entry.contentRect.height;
				const previous = pendingHeights.get(key) ?? rowHeights.get(key) ?? 0;
				if (height > 0 && Math.abs(previous - height) > 0.5) pendingHeights.set(key, height);
			}
			// ResizeObserver only records sizes; publishing in the next frame avoids
			// a measure/render feedback loop in WebKit.
			if (pendingHeights.size > 0 && measurementFrame === null) {
				measurementFrame = requestAnimationFrame(applyMeasuredRows);
			}
		});
		rowObserver.observe(node);
		return { destroy: () => rowObserver?.unobserve(node) };
	}

	$effect(() => {
		compact;
		showDates;
		untrack(() => {
			rowHeights.clear();
			pendingHeights.clear();
			if (measurementFrame !== null) cancelAnimationFrame(measurementFrame);
			measurementFrame = null;
			heightRevision++;
			void tick().then(() => {
				if (disposed || !virtualList || !rowObserver) return;
				for (const row of virtualList.querySelectorAll<HTMLDivElement>('.virtual-note-row')) {
					rowObserver.unobserve(row);
					rowObserver.observe(row);
				}
			});
		});
	});

	$effect(() => {
		$viewMode;
		calMonth;
		calYear;
		trashNotebooks.length;
		const element = virtualList;
		void tick().then(() => {
			if (!disposed && element) listOrigin = element.offsetTop;
		});
	});

	onDestroy(() => {
		disposed = true;
		refreshRequest++;
		cancelNoteNavigation(navigationToken);
		if (longPressTimer) clearTimeout(longPressTimer);
		rowObserver?.disconnect();
		if (measurementFrame !== null) cancelAnimationFrame(measurementFrame);
	});

	function onListScroll(e: Event) {
		const el = e.currentTarget as HTMLDivElement;
		if (el.clientHeight === 0) return;
		if (el.scrollHeight - el.scrollTop - el.clientHeight <= 1) stickToBottom = true;
		else if (el.scrollTop < scrollTop - 1) stickToBottom = false;
		scrollTop = el.scrollTop;
	}

	function clearSelection() {
		selectedPaths = new Set();
		lastClickedPath = null;
		batchMovePicker = false;
		multiSelectMode = false;
	}

	// Clear selection and reset scroll when view/notebook changes
	$effect(() => {
		const _ = [$viewMode, $activeNotebook, $activeTag];
		clearSelection();
		stickToBottom = false;
		scrollTop = 0;
		if (listContainer) listContainer.scrollTop = 0;
	});

	// Notes can also be removed from this list by a drop handled in the sidebar.
	$effect(() => {
		if (selectedPaths.size === 0) return;
		const availablePaths = new Set($notes.map((note) => note.path));
		const remaining = new Set([...selectedPaths].filter((path) => availablePaths.has(path)));
		if (remaining.size === selectedPaths.size) return;
		if (remaining.size === 0) {
			clearSelection();
		} else {
			selectedPaths = remaining;
		}
	});

	// Invalidate quickaccess cache when starred notes change (e.g. from Editor star toggle)
	$effect(() => {
		$quickAccessPaths;
		noteCache.delete('quickaccess');
		if ($viewMode === 'quickaccess') {
			refresh();
		}
	});

	// Track container height via ResizeObserver
	$effect(() => {
		if (!listContainer) return;
		const ro = new ResizeObserver((entries) => {
			containerHeight = entries[0].contentRect.height;
			if (virtualList) listOrigin = virtualList.offsetTop;
		});
		ro.observe(listContainer);
		return () => ro.disconnect();
	});

	interface FlatNotebook { name: string; path: string; depth: number; }

	function flattenNotebooks(nbs: import('$lib/types').NotebookEntry[], depth = 0): FlatNotebook[] {
		let result: FlatNotebook[] = [];
		for (const nb of nbs) {
			result.push({ name: nb.name, path: nb.path, depth });
			if (nb.children.length > 0) {
				result = result.concat(flattenNotebooks(nb.children, depth + 1));
			}
		}
		return result;
	}

	let flatNotebookList = $derived.by(() => {
		if (!movePickerNote && !batchMovePicker) return [];
		const all = flattenNotebooks($notebooks);
		if (movePickerNote) {
			const noteDir = movePickerNote.path.substring(0, movePickerNote.path.lastIndexOf('/'));
			return all.filter(nb => nb.path !== noteDir);
		}
		return all;
	});

	let viewTitle = $derived.by(() => {
		if ($viewMode === 'all') return 'All Notes';
		if ($viewMode === 'notebook') return $activeNotebook?.name ?? 'Notebook';
		if ($viewMode === 'tag') return `#${$activeTag}`;
		if ($viewMode === 'quickaccess') return 'Quick Access';
		if ($viewMode === 'daily') return 'Daily Notes';
		if ($viewMode === 'tasks') return 'Tasks';
		if ($viewMode === 'trash') return 'Trash';
		return 'Notes';
	});

	// Note list cache: avoids re-scanning when switching views
	let noteCache = new Map<string, NoteEntry[]>();
	let refreshRequest = 0;
	const pendingRefreshes = new Set<Map<string, NoteEntry>>();

	export function updateNote(note: NoteEntry): NoteEntry | undefined {
		if (!ownerIsCurrent()) return;
		let previous = $notes.find((entry) => entry.path === note.path);
		if (!previous) {
			for (const list of noteCache.values()) {
				previous = list.find((entry) => entry.path === note.path);
				if (previous) break;
			}
		}
		for (const pending of pendingRefreshes) pending.set(note.path, note);
		for (const [key, list] of noteCache) {
			noteCache.set(key, updateNoteList(list, note, key, $quickAccessPaths));
		}
		if ($viewMode !== 'tasks') {
			const updated = updateNoteList($notes, note, cacheKey(), $quickAccessPaths);
			if (updated !== $notes) $notes = updated;
		}
		return previous;
	}

	function cacheKey(): string {
		if ($viewMode === 'trash') return 'trash';
		if ($viewMode === 'daily') return 'daily';
		if ($viewMode === 'quickaccess') return 'quickaccess';
		if ($viewMode === 'tag' && $activeTag) return `tag:${$activeTag}`;
		if ($viewMode === 'notebook' && $activeNotebook) return `nb:${$activeNotebook.path}`;
		return 'all';
	}

	export async function refresh(invalidate = false) {
		if (!ownerIsCurrent()) return;
		const request = ++refreshRequest;
		const current = () => ownerIsCurrent() && request === refreshRequest;
		if (invalidate) noteCache.clear();
		if ($viewMode === 'tasks') { $notes = []; return; }
		const key = cacheKey();
		const mode = $viewMode;
		const notebookPath = $activeNotebook?.path ?? null;
		const tag = $activeTag;
		const vault = $appConfig?.active_vault;
		const cached = mode === 'trash' ? undefined : noteCache.get(key);
		if (cached) {
			$notes = cached;
			return;
		}

		// Replay watcher updates that arrive while a list read is in flight.
		const changes = new Map<string, NoteEntry>();
		pendingRefreshes.add(changes);
		try {
			let list: NoteEntry[];
			let allNotes: NoteEntry[] | undefined;
			if (mode === 'trash') {
				const trash = await getTrash();
				list = trash.notes;
				if (current()) trashNotebooks = trash.notebooks;
			} else if (mode === 'daily') {
				list = vault ? await getNotes(`${vault}/Daily`) : [];
			} else if (mode === 'quickaccess') {
				list = await getQuickAccess(ownerVault!);
			} else if (mode === 'tag' && tag) {
				allNotes = noteCache.get('all') ?? await getNotes(null);
				list = allNotes.filter((note) => note.meta.tags.includes(tag));
			} else {
				list = await getNotes(mode === 'notebook' ? notebookPath : null);
			}
			if (!current()) return;
			for (const note of changes.values()) {
				list = updateNoteList(list, note, key, $quickAccessPaths);
				if (allNotes) allNotes = updateNoteList(allNotes, note, 'all', $quickAccessPaths);
			}
			if (allNotes) noteCache.set('all', allNotes);
			noteCache.set(key, list);
			if (key !== cacheKey()) return;
			$notes = list;
			if (mode === 'daily') {
				dailyDates = new Set(list.flatMap((note) => {
					const match = note.path.split(/[\\/]/).pop()?.match(/^(\d{4}-\d{2}-\d{2})\.md$/);
					return match ? [match[1]] : [];
				}));
			}
		} catch (e) {
			console.error('Failed to refresh notes:', e);
		} finally {
			pendingRefreshes.delete(changes);
		}
	}

	async function selectNote(note: NoteEntry, token = beginNavigation()) {
		if (!navigationIsCurrent(token)) return;
		if ($activeNotePath === note.path) {
			if (isMobile) $mobileView = 'editor';
			return;
		}
		try {
			const content = await readNote(ownerVault!, note.path);
			if (!navigationIsCurrent(token)) return;
			if (!(await onBeforeNoteSwitch())) return;
			if (!navigationIsCurrent(token)) return;
			$activeNote = content;
			$activeNotePath = note.path;
			$editorDirty = false;
			onNoteSelected(note.path, content.content);
		} catch (e) {
			console.error('Failed to read note:', e);
		}
	}

	async function openTask(task: TaskItem) {
		const token = beginNavigation();
		if (!navigationIsCurrent(token)) return;
		try {
			const content = await readNote(ownerVault!, task.note_path);
			if (!navigationIsCurrent(token)) return;
			if (!(await onBeforeNoteSwitch())) return;
			if (!navigationIsCurrent(token)) return;
			$activeNote = content;
			$activeNotePath = task.note_path;
			$editorDirty = false;
			onNoteSelected(task.note_path, content.content, task);
		} catch (e) {
			console.error('Failed to open task note:', e);
		}
	}

	export async function handleCreateNote() {
		const token = beginNavigation();
		if (!navigationIsCurrent(token)) return;
		if ($viewMode === 'quickaccess' || $viewMode === 'trash') return;
		if ($viewMode === 'daily') {
			const today = new Date();
			const date = `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, '0')}-${String(today.getDate()).padStart(2, '0')}`;
			await handleDayClick(date, token);
			if (!navigationIsCurrent(token)) return;
			calMonth = today.getMonth();
			calYear = today.getFullYear();
			return;
		}
		// Target: active notebook, else the open note's folder, else root.
		let nbRelative: string | null = null;
		if ($viewMode === 'notebook' && $activeNotebook) {
			nbRelative = $activeNotebook.relative_path || null;
		} else if ($appConfig?.active_vault && $activeNotePath) {
			const vaultN = $appConfig.active_vault.replace(/\\/g, '/');
			const apN = $activeNotePath.replace(/\\/g, '/');
			if (apN.startsWith(vaultN + '/')) {
				const rel = apN.slice(vaultN.length + 1);
				const slash = rel.lastIndexOf('/');
				if (slash > 0) nbRelative = rel.slice(0, slash);
			}
		}
		try {
			if (!(await onBeforeNoteSwitch())) return;
			if (!navigationIsCurrent(token)) return;
			const entry = await createNote(ownerVault!, nbRelative, 'Untitled');
			if (!ownerIsCurrent()) return;
			if ($sortMode === 'custom') appendManualNoteOrder(entry.path);
			noteCache.clear();
			$notes = [entry, ...$notes];
			onNoteMoved();
			if (!navigationIsCurrent(token)) return;
			// The first save must use the exact file created on disk, not wait for a watcher echo.
			const content = await readNote(ownerVault!, entry.path);
			if (!navigationIsCurrent(token)) return;
			if (!(await onBeforeNoteSwitch())) return;
			if (!navigationIsCurrent(token)) return;
			$activeNote = content;
			$activeNotePath = entry.path;
			$editorDirty = false;
			onNoteSelected(entry.path, content.content);
			onNoteCreated(token);
		} catch (e) {
			console.error('Failed to create note:', e);
		}
	}

	async function handleDuplicate(note: NoteEntry) {
		const token = beginNavigation();
		if (!navigationIsCurrent(token)) return;
		contextMenu = null;
		try {
			if ($activeNotePath === note.path && $editorDirty) {
				const saved = await onBeforeNoteDuplicate();
				if (!saved) {
					console.error('Failed to save note before duplicating');
					return;
				}
			}
			if (!navigationIsCurrent(token)) return;

			const entry = await duplicateNote(ownerVault!, note.path);
			if (!ownerIsCurrent()) return;
			if ($sortMode === 'custom') appendManualNoteOrder(entry.path);
			noteCache.clear();
			if ($viewMode !== 'quickaccess') $notes = [entry, ...$notes];
			onNoteMoved();
			await selectNote(entry, token);
		} catch (e) {
			console.error('Failed to duplicate note:', e);
		}
	}

	function handleListDoubleClick(e: MouseEvent) {
		const target = e.target as HTMLElement | null;
		if (target?.closest('button, input, .note-item, .context-menu, .sort-menu, .batch-move-picker, .cal')) return;
		e.stopPropagation();
		if ($viewMode === 'tasks' || $viewMode === 'trash' || $viewMode === 'quickaccess') return;
		handleCreateNote();
	}

	async function handleRename(note: NoteEntry) {
		if (!editValue.trim() || editValue.trim() === note.meta.title) {
			editingNote = null;
			return;
		}
		const release = $activeNotePath === note.path ? await onBeforePathMutation() : () => {};
		if (!release) return;
		try {
			if (!ownerIsCurrent()) return;
			const newTitle = editValue.trim();
			const newPath = await renameNote(ownerVault!, note.path, newTitle);
			if (!ownerIsCurrent()) return;
			renameManualNoteOrderPath(note.path, newPath);
			noteCache.clear();
			editingNote = null;
			$notes = $notes.map(n =>
				n.path === note.path
					? { ...n, path: newPath, relative_path: n.relative_path.replace(/[^/]+$/, newTitle + '.md'), meta: { ...n.meta, title: newTitle } }
					: n
			);
			await onPathsMoved(note.path, newPath);
		} catch (e) {
			console.error('Failed to rename:', e);
		} finally {
			release();
		}
	}

	async function handleDelete(note: NoteEntry) {
		contextMenu = null;
		try {
			if ($activeNotePath === note.path && !(await onBeforeNoteSwitch())) return;
			if (!ownerIsCurrent()) return;
			await deleteNote(ownerVault!, note.path);
			if (!ownerIsCurrent()) return;
			removeManualNoteOrderPath(note.path);
			noteCache.clear();
			// Remove from local store immediately (avoid full vault re-scan)
			$notes = $notes.filter(n => n.path !== note.path);
			if ($activeNotePath === note.path) {
				$activeNote = null;
				$activeNotePath = null;
			}
		} catch (e) {
			console.error('Failed to delete:', e);
		}
	}


	async function handleRestore(note: NoteEntry) {
		contextMenu = null;
		try {
			await restoreNote(ownerVault!, note.path, null);
			if (!ownerIsCurrent()) return;
			noteCache.clear();
			await refresh();
		} catch (e) {
			console.error('Failed to restore:', e);
		}
	}

	async function handlePermanentDelete(note: NoteEntry) {
		contextMenu = null;
		try {
			await permanentDelete(ownerVault!, note.path);
			if (!ownerIsCurrent()) return;
			removeManualNoteOrderPath(note.path);
			noteCache.clear();
			$notes = $notes.filter(n => n.path !== note.path);
			if ($activeNotePath === note.path) {
				$activeNote = null;
				$activeNotePath = null;
			}
		} catch (e) {
			console.error('Failed to delete permanently:', e);
		}
	}

	async function handleRestoreNotebook(nb: TrashNotebookEntry) {
		trashBusy = nb.path;
		try {
			await restoreNotebook(ownerVault!, nb.path);
			if (!ownerIsCurrent()) return;
			noteCache.clear();
			await refresh();
		} catch (e) {
			console.error('Failed to restore notebook:', e);
		} finally {
			trashBusy = null;
		}
	}

	async function handleDeleteNotebook(nb: TrashNotebookEntry) {
		trashBusy = nb.path;
		try {
			await permanentDelete(ownerVault!, nb.path);
			if (!ownerIsCurrent()) return;
			noteCache.clear();
			trashNotebooks = trashNotebooks.filter(n => n.path !== nb.path);
		} catch (e) {
			console.error('Failed to delete notebook permanently:', e);
		} finally {
			trashBusy = null;
		}
	}

	async function updateMetadata(path: string, update: (meta: NoteMeta) => void) {
		if (!ownerIsCurrent()) return;
		const diskNote = $activeNotePath === path && $activeNote ? $activeNote : await readNote(ownerVault!, path);
		if (!ownerIsCurrent()) return;
		const current = $activeNotePath === path && $activeNote ? $activeNote : diskNote;
		const meta = { ...current.meta, tags: [...current.meta.tags] };
		update(meta);
		if ($activeNotePath === path && $activeNote) {
			if (!(await onUpdateActiveNote({ pinned: meta.pinned, tags: meta.tags }))) {
				throw new Error('Note still has unsaved changes');
			}
		} else {
			await saveNote(ownerVault!, path, meta, current.content, current.raw);
		}
		if (!ownerIsCurrent()) return;
		if (current.meta.tags.length !== meta.tags.length || current.meta.tags.some((tag) => !meta.tags.includes(tag))) {
			onTagsChanged(path, meta.tags);
		}
		let entry = $notes.find((note) => note.path === path);
		if (!entry) {
			for (const list of noteCache.values()) {
				entry = list.find((note) => note.path === path);
				if (entry) break;
			}
		}
		if (entry) updateNote({ ...entry, meta: { ...entry.meta, pinned: meta.pinned, tags: meta.tags } });
	}

	async function handleTogglePin(note: NoteEntry) {
		contextMenu = null;
		try {
			await updateMetadata(note.path, (meta) => { meta.pinned = !meta.pinned; });
		} catch (e) {
			console.error('Failed to toggle pin:', e);
		}
	}

	function openTagEdit(note: NoteEntry) {
		tagEditNote = note;
		tagEditTags = [...note.meta.tags];
		onRefreshTags();
	}

	async function addTagToNote(tag: string) {
		if (!tagEditNote || !tag.trim()) return;
		const cleaned = tag.trim().toLowerCase();
		if (tagEditTags.includes(cleaned)) return;
		tagEditTags = [...tagEditTags, cleaned];
		await saveTagsForNote(tagEditNote, tagEditTags);
	}

	async function removeTagFromNote(tag: string) {
		if (!tagEditNote) return;
		tagEditTags = tagEditTags.filter(t => t !== tag);
		await saveTagsForNote(tagEditNote, tagEditTags);
	}

	async function saveTagsForNote(note: NoteEntry, newTags: string[]) {
		try {
			await updateMetadata(note.path, (meta) => { meta.tags = newTags; });
		} catch (e) {
			console.error('Failed to save tags:', e);
		}
	}

	function openBatchTagEdit() {
		batchTagEdit = true;
		onRefreshTags();
		// Show tags common to ALL selected notes
		const selectedNotes = $notes.filter(n => selectedPaths.has(n.path));
		if (selectedNotes.length === 0) { tagEditTags = []; return; }
		tagEditTags = selectedNotes[0].meta.tags.filter(t =>
			selectedNotes.every(n => n.meta.tags.includes(t))
		);
	}

	async function addTagToBatch(tag: string) {
		if (!tag.trim()) return;
		const cleaned = tag.trim().toLowerCase();
		if (!tagEditTags.includes(cleaned)) tagEditTags = [...tagEditTags, cleaned];
		try {
			for (const path of selectedPaths) {
				await updateMetadata(path, (meta) => {
					if (!meta.tags.includes(cleaned)) meta.tags = [...meta.tags, cleaned];
				});
			}
		} catch (e) {
			console.error('Failed to add tag to batch:', e);
		}
	}

	async function removeTagFromBatch(tag: string) {
		tagEditTags = tagEditTags.filter(t => t !== tag);
		try {
			for (const path of selectedPaths) {
				await updateMetadata(path, (meta) => { meta.tags = meta.tags.filter((value) => value !== tag); });
			}
		} catch (e) {
			console.error('Failed to remove tag from batch:', e);
		}
	}

	async function handleAddQuickAccess(note: NoteEntry) {
		contextMenu = null;
		try {
			await addQuickAccess(ownerVault!, note.relative_path);
			if (!ownerIsCurrent()) return;
			const qaNotes = await getQuickAccess(ownerVault!);
			if (!ownerIsCurrent()) return;
			$quickAccessPaths = qaNotes.map(n => n.relative_path);
		} catch (e) {
			console.error('Failed to add to quick access:', e);
		}
	}

	async function handleRemoveQuickAccess(note: NoteEntry) {
		contextMenu = null;
		try {
			await removeQuickAccess(ownerVault!, note.relative_path);
			if (!ownerIsCurrent()) return;
			const qaNotes = await getQuickAccess(ownerVault!);
			if (!ownerIsCurrent()) return;
			$quickAccessPaths = qaNotes.map(n => n.relative_path);
			if ($viewMode === 'quickaccess') await refresh(true);
		} catch (e) {
			console.error('Failed to remove from quick access:', e);
		}
	}

	async function handleQaDrop(targetIndex: number) {
		if (qaDragFrom === null) { qaDragFrom = null; qaDragOver = null; return; }
		// Compute the actual insert position based on which half we're hovering
		let insertAt = qaDragHalf === 'bottom' ? targetIndex + 1 : targetIndex;
		if (qaDragFrom === insertAt || qaDragFrom + 1 === insertAt) {
			qaDragFrom = null; qaDragOver = null; return;
		}
		const arr = [...$sortedNotes];
		const [moved] = arr.splice(qaDragFrom, 1);
		if (insertAt > qaDragFrom) insertAt--;
		arr.splice(insertAt, 0, moved);
		$notes = arr;
		qaDragFrom = null;
		qaDragOver = null;
		try {
			await reorderQuickAccess(arr.map(n => n.relative_path));
		} catch (e) {
			console.error('Failed to reorder quick access:', e);
		}
	}

	function persistManualNoteOrder(orderedNotes: NoteEntry[]) {
		// Merge into (not replace) the order map so reordering notes in one view does
		// not wipe the saved custom order of other notebooks / tags / the all-notes view.
		const updates = Object.fromEntries(orderedNotes.map((note, index) => [note.path, index]));
		$noteOrder = { ...$noteOrder, ...updates };
	}

	function appendManualNoteOrder(path: string) {
		const nextIndex = Math.max(-1, ...Object.values($noteOrder)) + 1;
		$noteOrder = { ...$noteOrder, [path]: nextIndex };
	}

	function renameManualNoteOrderPath(oldPath: string, newPath: string) {
		if (!Object.hasOwn($noteOrder, oldPath)) return;
		const { [oldPath]: index, ...rest } = $noteOrder;
		$noteOrder = { ...rest, [newPath]: index };
	}

	function removeManualNoteOrderPath(path: string) {
		if (!Object.hasOwn($noteOrder, path)) return;
		const { [path]: _, ...rest } = $noteOrder;
		$noteOrder = rest;
	}

	function resetNoteDrag() {
		noteDragFrom = null;
		noteDragOver = null;
	}

	function handleCustomNoteDrop(targetIndex: number) {
		if (noteDragFrom === null || !canCustomReorder) {
			resetNoteDrag();
			return;
		}
		let insertAt = noteDragHalf === 'bottom' ? targetIndex + 1 : targetIndex;
		if (noteDragFrom === insertAt || noteDragFrom + 1 === insertAt) {
			resetNoteDrag();
			return;
		}
		const arr = [...$sortedNotes];
		const [moved] = arr.splice(noteDragFrom, 1);
		if (insertAt > noteDragFrom) insertAt--;
		arr.splice(insertAt, 0, moved);
		$notes = arr;
		persistManualNoteOrder(arr);
		resetNoteDrag();
	}

	async function handleMoveNote(note: NoteEntry, destPath: string) {
		contextMenu = null;
		movePickerNote = null;
		const release = $activeNotePath === note.path ? await onBeforePathMutation() : () => {};
		if (!release) return;
		try {
			if (!ownerIsCurrent()) return;
			const newPath = await moveNote(ownerVault!, note.path, destPath);
			if (!ownerIsCurrent()) return;
			renameManualNoteOrderPath(note.path, newPath);
			noteCache.clear();
			$notes = $notes.filter(n => n.path !== note.path);
			await onPathsMoved(note.path, newPath);
			if (ownerIsCurrent()) onNoteMoved();
		} catch (e) {
			console.error('Failed to move note:', e);
		} finally {
			release();
		}
	}

	function isFiled(note: NoteEntry): boolean {
		const vaultRoot = $appConfig?.active_vault?.replace(/\\/g, '/').replace(/\/$/, '');
		if (!vaultRoot) return false;
		const normalizedPath = note.path.replace(/\\/g, '/');
		const noteDir = normalizedPath.substring(0, normalizedPath.lastIndexOf('/'));
		return noteDir !== vaultRoot;
	}

	function unfileNote(note: NoteEntry) {
		const vaultRoot = $appConfig?.active_vault;
		if (vaultRoot) handleMoveNote(note, vaultRoot);
	}

	function clampMenu(x: number, y: number, menuWidth = 220, menuHeight = 400): { x: number; y: number } {
		if (x + menuWidth > window.innerWidth) x = window.innerWidth - menuWidth - 8;
		if (y + menuHeight > window.innerHeight) y = window.innerHeight - menuHeight - 8;
		if (x < 4) x = 4;
		if (y < 4) y = 4;
		return { x, y };
	}

	function onContextMenu(e: MouseEvent, note: NoteEntry) {
		e.preventDefault();
		const { x, y } = clampMenu(e.clientX, e.clientY);
		contextMenu = { x, y, note };
	}

	async function startRename(note: NoteEntry) {
		contextMenu = null;
		editingNote = note.path;
		editValue = note.meta.title;
		await tick();
		if (!ownerIsCurrent()) return;
		renameInput?.focus();
		renameInput?.select();
	}

	function getNotebookPath(note: NoteEntry): string {
		if ($viewMode === 'notebook') return '';
		const parts = note.relative_path.replace(/\.md$/, '').split('/');
		return parts.length > 1 ? parts.slice(0, -1).join('/') : '';
	}

	function handleNoteClick(e: MouseEvent, note: NoteEntry) {
		// Mobile: if long-press just fired, ignore the click
		if (isMobile && longPressTriggered) {
			longPressTriggered = false;
			e.preventDefault();
			return;
		}
		// Mobile: if in selection mode (or Android multi-select mode), tap toggles selection
		if (isMobile && (selectedPaths.size > 0 || multiSelectMode)) {
			const next = new Set(selectedPaths);
			if (next.has(note.path)) {
				next.delete(note.path);
			} else {
				next.add(note.path);
			}
			selectedPaths = next;
			return;
		}
		if (e.ctrlKey || e.metaKey) {
			// Toggle individual selection
			const next = new Set(selectedPaths);
			// On first Ctrl+click, also include the currently active note
			if (next.size === 0 && $activeNotePath && $activeNotePath !== note.path) {
				next.add($activeNotePath);
			}
			if (next.has(note.path)) {
				next.delete(note.path);
			} else {
				next.add(note.path);
			}
			selectedPaths = next;
			lastClickedPath = note.path;
			return;
		}
		if (e.shiftKey) {
			// Range select - use lastClickedPath or active note as anchor
			const anchor = lastClickedPath || $activeNotePath;
			if (!anchor) { selectNote(note); return; }
			const paths = $sortedNotes.map(n => n.path);
			const startIdx = paths.indexOf(anchor);
			const endIdx = paths.indexOf(note.path);
			if (startIdx >= 0 && endIdx >= 0) {
				const from = Math.min(startIdx, endIdx);
				const to = Math.max(startIdx, endIdx);
				const next = new Set(selectedPaths);
				for (let i = from; i <= to; i++) {
					next.add(paths[i]);
				}
				selectedPaths = next;
			}
			return;
		}
		clearSelection();
		selectNote(note);
	}

	// Batch operations
	async function handleBatchDelete() {
		const toDelete = new Set(selectedPaths);
		if ($activeNotePath && toDelete.has($activeNotePath) && !(await onBeforeNoteSwitch())) return;
		if (!ownerIsCurrent()) return;
		noteCache.clear();
		for (const path of toDelete) removeManualNoteOrderPath(path);
		$notes = $notes.filter(n => !toDelete.has(n.path));
		if ($activeNotePath && toDelete.has($activeNotePath)) {
			$activeNote = null;
			$activeNotePath = null;
		}
		clearSelection();
		await Promise.all([...toDelete].map(p => deleteNote(ownerVault!, p).catch(e => console.error('Failed to delete:', p, e))));
		if (!ownerIsCurrent()) return;
		onNoteMoved();
	}

	async function handleBatchMove(destPath: string) {
		const toMove = new Set(selectedPaths);
		const release = $activeNotePath && toMove.has($activeNotePath) ? await onBeforePathMutation() : () => {};
		if (!release) return;
		try {
			if (!ownerIsCurrent()) return;
			for (const path of toMove) {
				try {
					const newPath = await moveNote(ownerVault!, path, destPath);
					if (!ownerIsCurrent()) return;
					renameManualNoteOrderPath(path, newPath);
					await onPathsMoved(path, newPath);
					$notes = $notes.filter(note => note.path !== path);
				} catch (error) {
					console.error('Failed to move:', path, error);
				}
			}
			noteCache.clear();
			clearSelection();
			if (ownerIsCurrent()) onNoteMoved();
		} finally {
			release();
		}
	}

	async function handleBatchRestore() {
		const toRestore = [...selectedPaths];
		noteCache.clear();
		clearSelection();
		await Promise.all(toRestore.map(p => restoreNote(ownerVault!, p, null).catch(e => console.error('Failed to restore:', p, e))));
		await refresh();
		if (!ownerIsCurrent()) return;
		onNoteMoved();
	}

	async function handleBatchPermanentDelete() {
		const toDelete = new Set(selectedPaths);
		noteCache.clear();
		for (const path of toDelete) removeManualNoteOrderPath(path);
		$notes = $notes.filter(n => !toDelete.has(n.path));
		if ($activeNotePath && toDelete.has($activeNotePath)) {
			$activeNote = null;
			$activeNotePath = null;
		}
		clearSelection();
		await Promise.all([...toDelete].map(p => permanentDelete(ownerVault!, p).catch(e => console.error('Failed to delete:', p, e))));
	}

	function handleWindowClick(event: MouseEvent) {
		// Opening a submenu can detach the clicked button before window sees it.
		if (event.composedPath().some((node) => node instanceof Element && node.matches('.context-menu, .sort-menu'))) return;
		if (contextMenu) { contextMenu = null; movePickerNote = null; tagEditNote = null; batchTagEdit = false; }
		if (sortMenu) sortMenu = null;
	}

	function closeBatchMoveOverlay(event: MouseEvent) {
		if (event.target === event.currentTarget) batchMovePicker = false;
	}

	function closeBatchTagOverlay(event: MouseEvent) {
		if (event.target === event.currentTarget) batchTagEdit = false;
	}

	function openSortMenu(e: MouseEvent) {
		e.stopPropagation();
		const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
		const menuWidth = isMobile ? 220 : 180;
		let x = rect.left;
		if (x + menuWidth > window.innerWidth) x = window.innerWidth - menuWidth - 8;
		if (x < 4) x = 4;
		sortMenu = { x, y: rect.bottom + 4 };
	}

	function setSortMode(mode: SortMode) {
		if (mode === 'custom' && Object.keys($noteOrder).length === 0) {
			persistManualNoteOrder($sortedNotes);
		}
		$sortMode = mode;
		sortMenu = null;
	}
</script>

<svelte:window onclick={handleWindowClick} onkeydown={(e) => {
	if (e.key === 'Escape' && selectedPaths.size > 0) {
		clearSelection();
	}
	if ((e.ctrlKey || e.metaKey) && e.key === 'a' && $sortedNotes.length > 0) {
		// Only select all notes when Ctrl+A is not meant for a text field or either editor
		// (the source editor is CodeMirror, and a read-only editor is not contenteditable).
		const target = e.target as HTMLElement | null;
		const tag = target?.tagName;
		if (tag !== 'INPUT' && tag !== 'TEXTAREA' && tag !== 'SELECT' && !target?.isContentEditable
			&& !target?.closest('.ProseMirror, .cm-editor')) {
			e.preventDefault();
			selectedPaths = new Set($sortedNotes.map(n => n.path));
		}
	}
}} />

<div class="note-list" class:mobile={isMobile}>
	<div class="list-header">
		<span class="list-title">
			{#if $viewMode === 'tag' && $activeTag}
				<TagLabel name={$activeTag} size={16} />
			{:else}
				{viewTitle}
			{/if}
		</span>
		<div class="list-actions">
			{#if !isMobile}
				<button class="icon-btn" onclick={() => ($notelistCollapsed = true)} title={`Hide notes list (${modKey}+Shift+\\)`} aria-label="Hide notes list">
					<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						<line x1="8" y1="6" x2="20" y2="6" />
						<line x1="8" y1="12" x2="20" y2="12" />
						<line x1="8" y1="18" x2="20" y2="18" />
						<polyline points="5 8 3 12 5 16" />
					</svg>
				</button>
			{/if}
			{#if $viewMode !== 'tasks'}
			{#if isAndroid}
				<button class="icon-btn" class:active-toggle={multiSelectMode} onclick={() => { multiSelectMode = !multiSelectMode; if (!multiSelectMode) clearSelection(); }} title="Multi-select">
					<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						<rect x="3" y="3" width="7" height="7" rx="1" /><rect x="3" y="14" width="7" height="7" rx="1" />{#if multiSelectMode}<polyline points="6 6.5 7 7.5 9 5.5" stroke-width="2" /><polyline points="6 17.5 7 18.5 9 16.5" stroke-width="2" />{/if}<line x1="14" y1="6.5" x2="21" y2="6.5" /><line x1="14" y1="17.5" x2="21" y2="17.5" />
					</svg>
				</button>
			{/if}
			<button class="icon-btn" onclick={openSortMenu} title="Sort: {$sortMode}">
				<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<line x1="4" y1="6" x2="11" y2="6" /><line x1="4" y1="12" x2="15" y2="12" /><line x1="4" y1="18" x2="20" y2="18" />
				</svg>
			</button>
			{#if $viewMode === 'all' && ($sortMode === 'created' || $sortMode === 'modified')}
				<button class="icon-btn" class:active-toggle={$groupNotesByDate} onclick={() => { $groupNotesByDate = !$groupNotesByDate; }} title="Group by date">
					<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						<line x1="8" y1="6" x2="21" y2="6" /><line x1="8" y1="12" x2="21" y2="12" /><line x1="8" y1="18" x2="21" y2="18" /><line x1="3" y1="6" x2="3.01" y2="6" /><line x1="3" y1="12" x2="3.01" y2="12" /><line x1="3" y1="18" x2="3.01" y2="18" />
					</svg>
				</button>
			{/if}
			{#if $viewMode !== 'trash' && $viewMode !== 'quickaccess'}
				<button class={isMobile ? 'mobile-create-btn' : 'icon-btn'} onclick={handleCreateNote} title={`New note (${modKey}+N)`}>
					<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width={isMobile ? '3' : '2'} stroke-linecap="round" stroke-linejoin="round">
						<line x1="12" y1="5" x2="12" y2="19" /><line x1="5" y1="12" x2="19" y2="12" />
					</svg>
				</button>
			{/if}
			{/if}
		</div>
	</div>

	{#if selectedPaths.size > 0}
		<div class="selection-bar">
			<span class="selection-count">{selectedPaths.size} selected</span>
			<div class="selection-actions">
				{#if $viewMode === 'trash'}
					<button class="selection-action" onclick={handleBatchRestore} title="Restore selected">
						<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/></svg>
						Restore
					</button>
					<button class="selection-action danger" onclick={handleBatchPermanentDelete} title="Delete permanently">
						<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6l-2 14a2 2 0 0 1-2 2H9a2 2 0 0 1-2-2L5 6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
						Delete
					</button>
				{:else}
					<button class="selection-action" onclick={() => { batchMovePicker = true; }}>Move</button>
					<button class="selection-action" onclick={() => openBatchTagEdit()}>Tag</button>
					<button class="selection-action danger" onclick={handleBatchDelete}>Delete</button>
				{/if}
				<button class="selection-close" onclick={clearSelection} title="Clear selection">
					<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
				</button>
			</div>
		</div>
	{/if}

	{#if batchMovePicker}
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div class="batch-move-overlay" onclick={closeBatchMoveOverlay} onkeydown={(e) => { if (e.key === 'Escape') batchMovePicker = false; }}>
			<div class="batch-move-picker" role="dialog" aria-modal="true" aria-labelledby="batch-move-title" tabindex="-1">
				<div class="batch-move-header">
					<span id="batch-move-title">Move {selectedPaths.size} notes to...</span>
					<button class="selection-close" onclick={() => batchMovePicker = false} aria-label="Close move picker">
						<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
					</button>
				</div>
				{#if flatNotebookList.length === 0}
					<div class="context-empty">No notebooks available</div>
				{:else}
					<div class="move-picker-list">
						{#each flatNotebookList as nb}
							<button style="padding-left: {12 + nb.depth * 16}px" onclick={() => handleBatchMove(nb.path)}>
								<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
									<path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/>
								</svg>
								{nb.name}
							</button>
						{/each}
					</div>
				{/if}
			</div>
		</div>
	{/if}

	{#if batchTagEdit && !contextMenu}
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div class="batch-move-overlay" onclick={closeBatchTagOverlay} onkeydown={(e) => { if (e.key === 'Escape') batchTagEdit = false; }}>
			<div class="batch-move-picker" role="dialog" aria-modal="true" aria-labelledby="batch-tags-title" tabindex="-1">
				<div class="batch-move-header">
					<span id="batch-tags-title">Tags for {selectedPaths.size} notes</span>
					<button class="selection-close" onclick={() => batchTagEdit = false} aria-label="Close tag editor">
						<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
					</button>
				</div>
				<div class="tag-edit-section">
					<div class="tag-edit-input-row">
						<TagSuggestInput
							existing={tagEditTags}
							placeholder="Add tag to all..."
							onsubmit={(t) => addTagToBatch(t)}
							oncancel={() => batchTagEdit = false}
						/>
					</div>
					{#if tagEditTags.length > 0}
						<div class="tag-edit-list">
							{#each tagEditTags as tag, i (tagIterationKey(tag, i))}
								<div class="tag-edit-item">
									<span class="tag-edit-name"><TagLabel name={tag} size={12} /></span>
									<button class="tag-edit-remove" onclick={() => removeTagFromBatch(tag)} title="Remove from all">
										<svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
									</button>
								</div>
							{/each}
						</div>
					{:else}
						<div class="context-empty">No common tags</div>
					{/if}
				</div>
			</div>
		</div>
	{/if}

	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="list-content" class:tasks-mode={$viewMode === 'tasks'} bind:this={listContainer} onscroll={onListScroll} ondblclick={handleListDoubleClick}>
		{#if $viewMode === 'tasks'}
			<TasksView onOpenTask={openTask} onToggleTask={onToggleTask} onSetTaskPriority={onSetTaskPriority} onSetTaskDue={onSetTaskDue} />
		{/if}
		{#if $viewMode === 'daily'}
			<div class="cal">
				<div class="cal-nav">
					<button class="cal-nav-btn" onclick={calPrev} title="Previous month">
						<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="15 18 9 12 15 6"/></svg>
					</button>
					<button class="cal-title" onclick={calToday}>{calMonthNames[calMonth]} {calYear}</button>
					<button class="cal-nav-btn" onclick={calNext} title="Next month">
						<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="9 18 15 12 9 6"/></svg>
					</button>
				</div>
				<div class="cal-grid">
					{#each calDayNames as name}
						<div class="cal-head">{name}</div>
					{/each}
					{#each calDays() as cell}
						{#if cell.day === 0}
							<div class="cal-cell empty"></div>
						{:else}
							<button
								class="cal-cell"
								class:today={cell.current}
								class:has-note={dailyDates.has(cell.date)}
								class:active={$activeNotePath?.endsWith(`${cell.date}.md`) ?? false}
								onclick={() => handleDayClick(cell.date)}
							>
								{cell.day}
							</button>
						{/if}
					{/each}
				</div>
			</div>
		{/if}

		{#if $sortedNotes.length === 0 && $viewMode !== 'daily' && $viewMode !== 'tasks' && (!($viewMode === 'trash') || trashNotebooks.length === 0)}
			<div class="empty-state">
				{#if $viewMode === 'trash'}
					<p>Trash is empty</p>
				{:else if $viewMode === 'quickaccess'}
					<p>No starred notes</p>
				{:else}
					<p>No notes yet</p>
					<button class="btn-link" onclick={handleCreateNote}>Create your first note</button>
				{/if}
			</div>
		{/if}

		{#if $viewMode === 'trash' && trashNotebooks.length > 0}
			{#each trashNotebooks as nb (nb.path)}
				<div class="note-item trash-notebook" class:busy={trashBusy === nb.path}>
					<div class="note-title">
						<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="flex-shrink:0;opacity:0.6">
							<path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/>
						</svg>
						{nb.name}
					</div>
					{#if trashBusy === nb.path}
						<div class="note-preview trash-busy">
							<svg class="spinner" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><path d="M12 2a10 10 0 0 1 10 10"/></svg>
							Working...
						</div>
					{:else}
						<div class="note-preview">{nb.note_count} {nb.note_count === 1 ? 'note' : 'notes'}</div>
						<div class="trash-notebook-actions">
							<button class="trash-nb-btn restore" onclick={() => handleRestoreNotebook(nb)} title="Restore notebook">
								<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="1 4 1 10 7 10"/><path d="M3.51 15a9 9 0 105.64-11.36L1 10"/></svg>
								Restore
							</button>
							<button class="trash-nb-btn delete" onclick={() => handleDeleteNotebook(nb)} title="Delete permanently">
								<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14H6L5 6"/><path d="M10 11v6"/><path d="M14 11v6"/><path d="M9 6V4h6v2"/></svg>
								Delete
							</button>
						</div>
					{/if}
				</div>
			{/each}
		{/if}

		{#snippet noteRow(note: NoteEntry, noteIndex: number)}
			{#if editingNote === note.path}
				<div class="note-item active">
					<input
						type="text"
						class="rename-input"
						bind:this={renameInput}
						bind:value={editValue}
						onkeydown={(e) => {
							if (e.key === 'Enter') handleRename(note);
							if (e.key === 'Escape') editingNote = null;
						}}
						onblur={() => handleRename(note)}
					/>
				</div>
			{:else}
				<button
					class="note-item"
					class:active={$activeNotePath === note.path}
					class:selected={selectedPaths.has(note.path)}
					class:pinned={note.meta.pinned}
					class:compact={compact}
					class:qa-drag-above={$viewMode === 'quickaccess' && qaDragOver === noteIndex && qaDragFrom !== null && qaDragHalf === 'top'}
					class:qa-drag-below={$viewMode === 'quickaccess' && qaDragOver === noteIndex && qaDragFrom !== null && qaDragHalf === 'bottom'}
					class:note-drag-above={canCustomReorder && noteDragOver === noteIndex && noteDragFrom !== null && noteDragHalf === 'top'}
					class:note-drag-below={canCustomReorder && noteDragOver === noteIndex && noteDragFrom !== null && noteDragHalf === 'bottom'}
					title={$viewMode === 'quickaccess' ? note.path : (getNotebookPath(note) ? `${getNotebookPath(note)}/${note.meta.title}` : note.meta.title)}
					onclick={(e) => handleNoteClick(e, note)}
					onkeydown={(e) => {
						if (e.key === 'F2') {
							e.preventDefault();
							startRename(note);
						}
					}}
					ontouchstart={(e) => { if (isMobile && !isAndroid) handleTouchStart(e, note); }}
					ontouchmove={(e) => { if (isMobile && !isAndroid) handleTouchMove(e); }}
					ontouchend={() => { if (isMobile && !isAndroid) handleTouchEnd(); }}
					ontouchcancel={() => { if (isMobile && !isAndroid) handleTouchEnd(); }}
					oncontextmenu={(e) => {
						e.preventDefault();
						const pos = clampMenu(e.clientX, e.clientY);
						// If multi-selected and right-clicking a selected note, show batch menu
						if (selectedPaths.size > 1 && selectedPaths.has(note.path)) {
							contextMenu = { x: pos.x, y: pos.y, note };
							return;
						}
						// Otherwise clear selection and show single menu
						if (selectedPaths.size > 0 && !selectedPaths.has(note.path)) {
							clearSelection();
						}
						contextMenu = { x: pos.x, y: pos.y, note };
					}}
					draggable="true"
					ondragstart={(e) => {
						if ($viewMode === 'quickaccess') {
							qaDragFrom = noteIndex;
							e.dataTransfer!.setData('text/plain', note.path);
							e.dataTransfer!.effectAllowed = 'move';
							return;
						}
						if (canCustomReorder) {
							noteDragFrom = noteIndex;
							e.dataTransfer!.setData('text/plain', note.path);
							e.dataTransfer!.effectAllowed = 'move';
							return;
						}
						const dragPaths = selectedPaths.size > 1 && selectedPaths.has(note.path)
							? selectedPaths
							: [note.path];
						e.dataTransfer!.setData('text/plain', encodeNoteDragPaths(dragPaths));
						e.dataTransfer!.effectAllowed = 'move';
					}}
					ondragover={(e) => {
						if ($viewMode === 'quickaccess' && qaDragFrom !== null) {
							e.preventDefault();
							e.dataTransfer!.dropEffect = 'move';
							const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
							qaDragHalf = e.clientY < rect.top + rect.height / 2 ? 'top' : 'bottom';
							qaDragOver = noteIndex;
						} else if (canCustomReorder && noteDragFrom !== null) {
							e.preventDefault();
							e.dataTransfer!.dropEffect = 'move';
							const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
							noteDragHalf = e.clientY < rect.top + rect.height / 2 ? 'top' : 'bottom';
							noteDragOver = noteIndex;
						}
					}}
					ondragleave={() => {
						if (qaDragOver === noteIndex) qaDragOver = null;
						if (noteDragOver === noteIndex) noteDragOver = null;
					}}
					ondrop={(e) => {
						if ($viewMode === 'quickaccess' && qaDragFrom !== null) {
							e.preventDefault();
							handleQaDrop(noteIndex);
						} else if (canCustomReorder && noteDragFrom !== null) {
							e.preventDefault();
							handleCustomNoteDrop(noteIndex);
						}
					}}
					ondragend={() => { qaDragFrom = null; qaDragOver = null; resetNoteDrag(); }}
				>
					{#if compact}
						<div class="note-compact-row">
							<span class="note-title">
								{#if note.meta.pinned}
									<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="pin-icon">
										<path d="M12 17v5"/><path d="M9 2h6l-1 7h4l-2 4H8l-2-4h4L9 2z"/>
									</svg>
								{/if}
								{#if $viewMode !== 'quickaccess' && $quickAccessPaths.includes(note.relative_path)}
									<svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="quickaccess-icon">
										<polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"/>
									</svg>
								{/if}
								{note.meta.title}
							</span>
							{#if getNotebookPath(note)}
								<span class="note-notebook">{getNotebookPath(note)}</span>
							{/if}
							{#if showDates}<span class="note-date-compact" title={`Created ${formatDate(note.meta.created)}\nModified ${formatDate(note.meta.modified)}`}>{formatRelativeTime($sortMode === 'created' ? note.meta.created : note.meta.modified)}</span>{/if}
						</div>
					{:else}
						<div class="note-title">
							{#if note.meta.pinned}
								<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="pin-icon">
									<path d="M12 17v5"/><path d="M9 2h6l-1 7h4l-2 4H8l-2-4h4L9 2z"/>
								</svg>
							{/if}
							{#if $viewMode !== 'quickaccess' && $quickAccessPaths.includes(note.relative_path)}
								<svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="quickaccess-icon">
									<polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"/>
								</svg>
							{/if}
							{note.meta.title}
						</div>
						<div class="note-preview">{note.preview}</div>
						<div class="note-meta">
							{#if getNotebookPath(note) || showDates}
							<span class="note-context">
								{#if getNotebookPath(note)}
									<span class="note-notebook">{getNotebookPath(note)}</span>
									{#if showDates}<span class="note-meta-sep">&middot;</span>{/if}
								{/if}
								{#if showDates}<span class="note-date" title={`Created ${formatDate(note.meta.created)}\nModified ${formatDate(note.meta.modified)}`}>{formatRelativeTime($sortMode === 'created' ? note.meta.created : note.meta.modified)}</span>{/if}
							</span>
							{/if}
							{#if note.meta.tags.length > 0}
								<span class="note-tags">
									{#each note.meta.tags.slice(0, 3) as tag, i (tagIterationKey(tag, i))}
										<TagLabel name={tag} size={10} tone="accent" />
									{/each}
								</span>
							{/if}
						</div>
				{/if}
				{#if $viewMode === 'trash'}
					<span class="trash-row-actions">
						<span class="trash-row-btn" role="button" tabindex="0" onclick={(e) => { e.stopPropagation(); handleRestore(note); }} title="Restore" onkeydown={(e) => { if (e.key === 'Enter') { e.stopPropagation(); handleRestore(note); } }}>
							<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/></svg>
						</span>
						<span class="trash-row-btn danger" role="button" tabindex="0" onclick={(e) => { e.stopPropagation(); handlePermanentDelete(note); }} title="Delete permanently" onkeydown={(e) => { if (e.key === 'Enter') { e.stopPropagation(); handlePermanentDelete(note); } }}>
							<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6l-2 14a2 2 0 0 1-2 2H9a2 2 0 0 1-2-2L5 6"/><path d="M10 11v6"/><path d="M14 11v6"/><path d="M9 6V4h6v2"/></svg>
						</span>
					</span>
				{/if}
			</button>
			{/if}
		{/snippet}

		<div class="virtual-note-rows" bind:this={virtualList}>
			<div style:height={`${windowRows.top}px`}></div>
			{#each visibleRows as row, index (row.key)}
				<div class="virtual-note-row" data-row-key={row.key} use:measureRow>
					{#if row.kind === 'section'}
						<div class="note-list-section" class:first-section={windowRows.start + index === 0}>{row.label}</div>
					{:else}
						{@render noteRow(row.note, row.noteIndex)}
					{/if}
				</div>
			{/each}
			<div style:height={`${windowRows.bottom}px`}></div>
		</div>
	</div>
</div>

{#if contextMenu}
	<div class="context-menu" class:mobile={isMobile} style="left: {contextMenu.x}px; top: {contextMenu.y}px" role="group" aria-label="Note actions">
		{#if selectedPaths.size > 1 && selectedPaths.has(contextMenu.note.path)}
			<!-- Batch context menu -->
			{#if $viewMode === 'trash'}
				<button onclick={() => { contextMenu = null; handleBatchRestore(); }}>
					Restore {selectedPaths.size} notes
				</button>
				<button class="danger" onclick={() => { contextMenu = null; handleBatchPermanentDelete(); }}>
					Delete {selectedPaths.size} permanently
				</button>
			{:else if batchTagEdit}
				<button class="move-back" onclick={() => batchTagEdit = false}>
					<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						<polyline points="15 18 9 12 15 6"/>
					</svg>
					Tags for {selectedPaths.size} notes
				</button>
				<div class="context-sep"></div>
				<div class="tag-edit-section">
					<div class="tag-edit-input-row">
						<TagSuggestInput
							existing={tagEditTags}
							placeholder="Add tag to all..."
							onsubmit={(t) => addTagToBatch(t)}
							oncancel={() => batchTagEdit = false}
						/>
					</div>
					{#if tagEditTags.length > 0}
						<div class="tag-edit-list">
							{#each tagEditTags as tag, i (tagIterationKey(tag, i))}
								<div class="tag-edit-item">
									<span class="tag-edit-name"><TagLabel name={tag} size={12} /></span>
									<button class="tag-edit-remove" onclick={() => removeTagFromBatch(tag)} title="Remove from all">
										<svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
									</button>
								</div>
							{/each}
						</div>
					{:else}
						<div class="context-empty">No common tags</div>
					{/if}
				</div>
			{:else}
				<button onclick={() => openBatchTagEdit()}>
					<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						<path d="M20.59 13.41l-7.17 7.17a2 2 0 01-2.83 0L2 12V2h10l8.59 8.59a2 2 0 010 2.82z"/><line x1="7" y1="7" x2="7.01" y2="7"/>
					</svg>
					Tags...
				</button>
				<button onclick={() => { contextMenu = null; batchMovePicker = true; }}>
					<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						<path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/>
					</svg>
					Move {selectedPaths.size} notes...
				</button>
				<button class="danger" onclick={() => { contextMenu = null; handleBatchDelete(); }}>
					Delete {selectedPaths.size} notes
				</button>
			{/if}
		{:else if $viewMode === 'trash'}
			<button onclick={() => handleRestore(contextMenu!.note)}>Restore</button>
			<button class="danger" onclick={() => handlePermanentDelete(contextMenu!.note)}>Delete Permanently</button>
		{:else if tagEditNote}
			<button class="move-back" onclick={() => tagEditNote = null}>
				<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<polyline points="15 18 9 12 15 6"/>
				</svg>
				Tags
			</button>
			<div class="context-sep"></div>
			<div class="tag-edit-section">
				<div class="tag-edit-input-row">
					<TagSuggestInput
						existing={tagEditTags}
						placeholder="Add tag..."
						onsubmit={(t) => addTagToNote(t)}
						oncancel={() => tagEditNote = null}
					/>
				</div>
				{#if tagEditTags.length > 0}
					<div class="tag-edit-list">
						{#each tagEditTags as tag, i (tagIterationKey(tag, i))}
							<div class="tag-edit-item">
								<span class="tag-edit-name"><TagLabel name={tag} size={12} /></span>
								<button class="tag-edit-remove" onclick={() => removeTagFromNote(tag)} title="Remove tag">
									<svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
								</button>
							</div>
						{/each}
					</div>
				{:else}
					<div class="context-empty">No tags</div>
				{/if}
			</div>
		{:else if movePickerNote}
			<button class="move-back" onclick={() => movePickerNote = null}>
				<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<polyline points="15 18 9 12 15 6"/>
				</svg>
				Move to...
			</button>
			<div class="context-sep"></div>
			{#if flatNotebookList.length === 0}
				<div class="context-empty">No other notebooks</div>
			{:else}
				<div class="move-picker-list">
					{#each flatNotebookList as nb}
						<button style="padding-left: {12 + nb.depth * 16}px" onclick={() => handleMoveNote(movePickerNote!, nb.path)}>
							<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
								<path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/>
							</svg>
							{nb.name}
						</button>
					{/each}
				</div>
			{/if}
		{:else}
			<button onclick={() => handleTogglePin(contextMenu!.note)}>
				<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<path d="M12 17v5"/><path d="M9 2h6l-1 7h4l-2 4H8l-2-4h4L9 2z"/>
				</svg>
				{contextMenu.note.meta.pinned ? 'Unpin Note' : 'Pin Note'}
			</button>
			<button onclick={() => openTagEdit(contextMenu!.note)}>
				<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<path d="M20.59 13.41l-7.17 7.17a2 2 0 01-2.83 0L2 12V2h10l8.59 8.59a2 2 0 010 2.82z"/><line x1="7" y1="7" x2="7.01" y2="7"/>
				</svg>
				Tags...
			</button>
			{#if $quickAccessPaths.includes(contextMenu.note.relative_path)}
				<button onclick={() => handleRemoveQuickAccess(contextMenu!.note)}>
					<svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						<polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" />
					</svg>
					Remove from Quick Access
				</button>
			{:else}
				<button onclick={() => handleAddQuickAccess(contextMenu!.note)}>
					<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						<polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" />
					</svg>
					Add to Quick Access
				</button>
			{/if}
			<button onclick={() => { const n = contextMenu!.note; contextMenu = null; openNoteWindow(ownerVault!, n.path, n.meta.title); }}>
				<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<path d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/>
				</svg>
				Open in New Window
			</button>
			{#if !isMobile}
			<button onclick={async () => { const n = contextMenu!.note; contextMenu = null; try { await revealFile(n.path); } catch (e) { console.error('Failed to reveal in file manager:', e); } }}>
				<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/>
				</svg>
				Show in File Manager
			</button>
			{/if}
			<button onclick={() => startRename(contextMenu!.note)}>
				<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<path d="M17 3a2.85 2.85 0 114 4L7.5 20.5 2 22l1.5-5.5Z"/><path d="m15 5 4 4"/>
				</svg>
				Rename
			</button>
			<button onclick={() => handleDuplicate(contextMenu!.note)}>
				<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<rect x="8" y="8" width="13" height="13" rx="2"/><path d="M16 8V5a2 2 0 00-2-2H5a2 2 0 00-2 2v9a2 2 0 002 2h3"/>
				</svg>
				Duplicate Note
			</button>
			<button onclick={() => { movePickerNote = contextMenu!.note; }}>
				<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/>
				</svg>
				Move to...
			</button>
			{#if isFiled(contextMenu.note)}
			<button onclick={() => unfileNote(contextMenu!.note)}>
				<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/><path d="M9 13h6"/>
				</svg>
				Unfile Note
			</button>
			{/if}
			{#if !isMobile}
			<button onclick={async () => { const n = contextMenu!.note; contextMenu = null; await selectNote(n); setTimeout(() => window.print(), 300); }}>
				<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="16" y1="13" x2="8" y2="13"/><line x1="16" y1="17" x2="8" y2="17"/><polyline points="10 9 9 9 8 9"/>
				</svg>
				Print / Export to PDF
			</button>
			{/if}
			<button class="danger" onclick={() => handleDelete(contextMenu!.note)}>
				<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/>
				</svg>
				Move to Trash
			</button>
		{/if}
	</div>
{/if}

{#if sortMenu}
	<div class="sort-menu" class:mobile={isMobile} style="left: {sortMenu.x}px; top: {sortMenu.y}px">
		<div class="sort-menu-title">Sort by</div>
		<button class:active={$sortMode === 'modified'} onclick={() => setSortMode('modified')}>
			<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
				<circle cx="12" cy="12" r="10" />
				<polyline points="12 6 12 12 16 14" />
			</svg>
			Last Modified
			{#if $sortMode === 'modified'}<span class="sort-check">&#10003;</span>{/if}
		</button>
		<button class:active={$sortMode === 'created'} onclick={() => setSortMode('created')}>
			<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
				<rect x="3" y="4" width="18" height="18" rx="2" ry="2" />
				<line x1="16" y1="2" x2="16" y2="6" />
				<line x1="8" y1="2" x2="8" y2="6" />
				<line x1="3" y1="10" x2="21" y2="10" />
			</svg>
			Date Created
			{#if $sortMode === 'created'}<span class="sort-check">&#10003;</span>{/if}
		</button>
		<button class:active={$sortMode === 'title'} onclick={() => setSortMode('title')}>
			<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
				<line x1="4" y1="6" x2="20" y2="6" />
				<line x1="4" y1="12" x2="14" y2="12" />
				<line x1="4" y1="18" x2="18" y2="18" />
			</svg>
			Title
			{#if $sortMode === 'title'}<span class="sort-check">&#10003;</span>{/if}
		</button>
		<button class:active={$sortMode === 'custom'} onclick={() => setSortMode('custom')}>
			<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
				<line x1="8" y1="6" x2="21" y2="6" />
				<line x1="8" y1="12" x2="21" y2="12" />
				<line x1="8" y1="18" x2="21" y2="18" />
				<circle cx="4" cy="6" r="1" />
				<circle cx="4" cy="12" r="1" />
				<circle cx="4" cy="18" r="1" />
			</svg>
			Custom
			{#if $sortMode === 'custom'}<span class="sort-check">&#10003;</span>{/if}
		</button>
	</div>
{/if}

<style>
	.note-list {
		display: flex;
		flex-direction: column;
		height: 100%;
		background: var(--bg-primary);
		border-right: 1px solid var(--border-color);
		user-select: none;
	}

	.list-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 10px 12px;
		border-bottom: 1px solid var(--border-light);
		flex-shrink: 0;
	}

	.list-title {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		min-width: 0;
		font-weight: 600;
		font-size: 14px;
		color: var(--text-primary);
	}

	.list-actions {
		display: flex;
		align-items: center;
		gap: 2px;
	}

	.icon-btn {
		background: none;
		border: none;
		color: var(--text-tertiary);
		cursor: pointer;
		padding: 4px;
		border-radius: 4px;
		display: flex;
		align-items: center;
	}

	.icon-btn:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.icon-btn.active-toggle {
		color: var(--accent);
		background: color-mix(in srgb, var(--accent) 15%, transparent);
	}

	.list-content {
		position: relative;
		flex: 1;
		overflow-y: auto;
		padding: 4px;
	}

	.virtual-note-row {
		display: flow-root;
	}

	.note-list-section {
		padding: 14px 12px 4px;
		font-size: 0.7rem;
		font-weight: 700;
		letter-spacing: 0.06em;
		text-transform: uppercase;
		color: var(--accent);
		border-bottom: 1px solid var(--border-light);
		margin-bottom: 2px;
		user-select: none;
	}
	.note-list-section.first-section {
		padding-top: 6px;
	}

	.note-item {
		display: block;
		width: 100%;
		padding: 10px 12px;
		border: none;
		background: none;
		border-radius: 6px;
		cursor: pointer;
		text-align: left;
		margin-bottom: 1px;
		contain: content;
		position: relative;
	}

	.note-item:hover {
		background: var(--bg-hover);
	}

	.note-item.active {
		background: var(--accent-light);
	}

	.note-item.selected {
		background: color-mix(in srgb, var(--accent) 12%, transparent);
	}

	.note-item.selected:hover {
		background: color-mix(in srgb, var(--accent) 18%, transparent);
	}

	.note-item.selected.active {
		background: var(--accent-light);
	}

	/* Selection bar */
	.selection-bar {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 6px 12px;
		background: color-mix(in srgb, var(--accent) 10%, var(--bg-primary));
		border-bottom: 1px solid var(--border-light);
		flex-shrink: 0;
	}

	.selection-count {
		font-size: 12px;
		font-weight: 600;
		color: var(--text-accent);
	}

	.selection-actions {
		display: flex;
		align-items: center;
		gap: 4px;
	}

	.selection-action {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		background: none;
		border: 1px solid var(--border-color);
		color: var(--text-primary);
		font-size: 11px;
		padding: 3px 10px;
		border-radius: 4px;
		cursor: pointer;
		font-weight: 500;
	}

	.selection-action:hover {
		background: var(--bg-hover);
	}

	.trash-row-actions {
		position: absolute;
		right: 6px;
		top: 50%;
		transform: translateY(-50%);
		display: none;
		gap: 2px;
	}
	.note-item:hover .trash-row-actions {
		display: flex;
	}
	.trash-row-btn {
		cursor: pointer;
		padding: 4px;
		border-radius: 4px;
		color: var(--text-secondary);
		display: flex;
		align-items: center;
	}
	.trash-row-btn:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}
	.trash-row-btn.danger:hover {
		color: var(--danger);
		background: color-mix(in srgb, var(--danger) 10%, transparent);
	}
	.note-list.mobile .trash-row-actions {
		display: flex;
	}

	.selection-action.danger {
		color: var(--danger);
		border-color: color-mix(in srgb, var(--danger) 30%, var(--border-color));
	}

	.selection-action.danger:hover {
		background: color-mix(in srgb, var(--danger) 10%, transparent);
	}

	.selection-close {
		background: none;
		border: none;
		color: var(--text-tertiary);
		cursor: pointer;
		padding: 4px;
		border-radius: 4px;
		display: flex;
		align-items: center;
	}

	.selection-close:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	/* Batch move overlay */
	.batch-move-overlay {
		position: fixed;
		inset: 0;
		background: rgba(0, 0, 0, 0.3);
		z-index: 900;
		display: flex;
		align-items: center;
		justify-content: center;
	}

	.batch-move-picker {
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 10px;
		box-shadow: var(--shadow-lg);
		min-width: 260px;
		max-width: 340px;
		max-height: 400px;
		display: flex;
		flex-direction: column;
		overflow: hidden;
	}

	.batch-move-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 10px 12px;
		border-bottom: 1px solid var(--border-light);
		font-size: 13px;
		font-weight: 600;
		color: var(--text-primary);
	}

	.batch-move-picker .move-picker-list {
		flex: 1;
		overflow-y: auto;
		padding: 4px;
	}

	.batch-move-picker .move-picker-list button {
		display: flex;
		align-items: center;
		gap: 8px;
		width: 100%;
		padding: 7px 12px;
		border: none;
		background: none;
		color: var(--text-primary);
		font-size: 13px;
		cursor: pointer;
		border-radius: 6px;
		text-align: left;
	}

	.batch-move-picker .move-picker-list button:hover {
		background: var(--bg-hover);
	}

	.note-title {
		font-weight: 500;
		font-size: inherit;
		color: var(--text-primary);
		display: flex;
		align-items: center;
		gap: 4px;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.note-item.compact {
		padding: 6px 12px;
	}

	.note-compact-row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
	}

	.note-compact-row .note-title {
		flex: 1;
		min-width: 0;
	}

	.note-date-compact {
		font-size: 11px;
		color: var(--text-tertiary);
		flex-shrink: 0;
		white-space: nowrap;
	}

	.pin-icon {
		color: var(--text-accent);
		flex-shrink: 0;
	}

	.quickaccess-icon {
		color: var(--text-accent);
		flex-shrink: 0;
	}

	.note-preview {
		font-size: 0.85em;
		color: var(--text-tertiary);
		margin-top: 2px;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.note-meta {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 4px 8px;
		margin-top: 4px;
	}

	.note-context {
		display: inline-flex;
		align-items: center;
		gap: 8px;
		min-width: 0;
		max-width: 100%;
	}

	.note-notebook {
		font-size: 0.78em;
		color: var(--text-tertiary);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		max-width: 140px;
	}

	.note-meta-sep {
		color: var(--text-tertiary);
		font-size: 0.78em;
		flex-shrink: 0;
	}

	.note-compact-row .note-notebook {
		font-size: 10px;
		max-width: 80px;
		flex-shrink: 0;
	}

	.note-date {
		font-size: 0.78em;
		color: var(--text-tertiary);
		flex-shrink: 0;
		white-space: nowrap;
	}

	.note-tags {
		display: flex;
		flex-wrap: wrap;
		min-width: 0;
		max-width: 100%;
		gap: 4px;
	}

	.rename-input {
		width: 100%;
		padding: 4px 8px;
		border: 1px solid var(--accent);
		border-radius: 4px;
		background: var(--bg-primary);
		color: var(--text-primary);
		font-size: inherit;
		outline: none;
	}

	.empty-state {
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		padding: 48px 16px;
		color: var(--text-tertiary);
		font-size: 13px;
	}

	.btn-link {
		background: none;
		border: none;
		color: var(--text-accent);
		cursor: pointer;
		font-size: 13px;
		margin-top: 8px;
	}

	.btn-link:hover {
		text-decoration: underline;
	}

	.context-menu {
		position: fixed;
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 8px;
		box-shadow: var(--shadow-md);
		padding: 4px;
		z-index: 1000;
		min-width: 160px;
	}

	.context-menu button {
		display: flex;
		align-items: center;
		gap: 8px;
		width: 100%;
		padding: 6px 12px;
		border: none;
		background: none;
		color: var(--text-primary);
		font-size: 13px;
		cursor: pointer;
		border-radius: 4px;
		text-align: left;
	}

	.context-menu button:hover {
		background: var(--bg-hover);
	}

	.context-menu button.danger {
		color: var(--danger);
	}

	.context-menu button.danger:hover {
		background: color-mix(in srgb, var(--danger) 10%, transparent);
	}

	.context-sep {
		height: 1px;
		background: var(--border-color);
		margin: 4px 8px;
	}

	.context-empty {
		padding: 8px 12px;
		color: var(--text-tertiary);
		font-size: 12px;
		text-align: center;
	}

	.move-back {
		font-weight: 600;
	}

	.move-picker-list {
		max-height: 240px;
		overflow-y: auto;
	}

	.move-picker-list::-webkit-scrollbar {
		width: 4px;
	}

	.move-picker-list::-webkit-scrollbar-thumb {
		background: var(--text-tertiary);
		border-radius: 2px;
	}

	.tag-edit-section {
		padding: 4px 0;
	}

	.tag-edit-input-row {
		padding: 2px 8px 4px;
	}

	.tag-edit-list {
		max-height: 160px;
		overflow-y: auto;
	}

	.tag-edit-item {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 3px 12px;
		font-size: 12px;
	}

	.tag-edit-name {
		color: var(--text-secondary);
	}

	.tag-edit-remove {
		background: none;
		border: none;
		color: var(--text-tertiary);
		cursor: pointer;
		padding: 2px;
		border-radius: 3px;
		display: flex;
		align-items: center;
	}

	.tag-edit-remove:hover {
		color: var(--danger, #e53e3e);
		background: color-mix(in srgb, var(--danger, #e53e3e) 10%, transparent);
	}

	.sort-menu {
		position: fixed;
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 10px;
		box-shadow: var(--shadow-lg);
		padding: 4px;
		z-index: 1000;
		min-width: 180px;
	}

	.sort-menu-title {
		padding: 6px 12px 4px;
		font-size: 11px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--text-tertiary);
	}

	.sort-menu button {
		display: flex;
		align-items: center;
		gap: 8px;
		width: 100%;
		padding: 7px 12px;
		border: none;
		background: none;
		color: var(--text-primary);
		font-size: 13px;
		cursor: pointer;
		border-radius: 6px;
		text-align: left;
	}

	.sort-menu button:hover {
		background: var(--bg-hover);
	}

	.sort-menu button.active {
		color: var(--text-accent);
	}

	.sort-check {
		margin-left: auto;
		color: var(--accent);
		font-size: 14px;
	}

	.note-item.qa-drag-above,
	.note-item.note-drag-above {
		border-top: 2px solid var(--accent);
	}

	.note-item.qa-drag-below,
	.note-item.note-drag-below {
		border-bottom: 2px solid var(--accent);
	}

	/* ═══ MOBILE (class-based for Android high-DPI) ═══ */
	.note-list.mobile {
		border-right: none;
	}

	.note-list.mobile .list-header {
		padding: 12px 16px;
	}

	.note-list.mobile .list-actions {
		gap: 10px;
	}

	.note-list.mobile .list-title {
		font-size: 16px;
	}

	.note-list.mobile .icon-btn {
		min-width: 44px;
		min-height: 44px;
		padding: 10px;
	}

	.note-list.mobile .list-content {
		padding: 4px 8px 180px;
	}

	.note-list.mobile .list-content.tasks-mode {
		padding-bottom: 0;
		overflow: hidden;
	}

	.note-list.mobile .note-item {
		padding: 14px 16px;
		min-height: 56px;
		border-radius: 10px;
		margin-bottom: 2px;
	}

	.note-list.mobile .note-item.compact {
		padding: 10px 16px;
		min-height: 48px;
	}

	.note-list.mobile .note-title {
		font-size: 15px;
	}

	.note-list.mobile .note-preview {
		font-size: 13px;
		margin-top: 4px;
	}

	.note-list.mobile .note-meta {
		margin-top: 6px;
	}

	.note-list.mobile .note-date {
		font-size: 12px;
	}

	.note-list.mobile :global(.tag-label.tone-accent) {
		font-size: 11px;
		padding: 2px 6px;
	}

	.note-list.mobile .rename-input {
		padding: 10px 12px;
		font-size: 15px;
	}

	.note-list.mobile .empty-state {
		padding: 64px 24px;
		font-size: 15px;
	}

	.note-list.mobile .btn-link {
		font-size: 15px;
	}

	.note-list.mobile .selection-bar {
		padding: 8px 16px;
		border-radius: 12px;
		margin: 4px 8px;
		border-bottom: none;
	}

	.note-list.mobile .selection-count {
		font-size: 13px;
	}

	.note-list.mobile .selection-action {
		padding: 6px 14px;
		font-size: 13px;
		min-height: 36px;
		border-radius: 10px;
	}

	.context-menu.mobile {
		min-width: 220px;
		border-radius: 12px;
		padding: 6px;
		max-height: calc(100dvh - 16px);
		overflow-y: auto;
	}

	.context-menu.mobile button {
		padding: 12px 16px;
		font-size: 15px;
		min-height: 44px;
		border-radius: 8px;
	}

	.sort-menu.mobile {
		min-width: 220px;
		border-radius: 12px;
		padding: 6px;
	}

	.sort-menu.mobile button {
		padding: 12px 16px;
		font-size: 15px;
		min-height: 44px;
		border-radius: 8px;
	}

	.note-list.mobile .batch-move-picker {
		min-width: 80vw;
		max-width: 90vw;
		max-height: 60vh;
	}

	.note-list.mobile .batch-move-header {
		padding: 14px 16px;
		font-size: 15px;
	}

	.note-list.mobile .batch-move-picker .move-picker-list button {
		padding: 12px 16px;
		font-size: 15px;
		min-height: 44px;
	}

	/* Mobile create note button (round, accent-colored) */
	.mobile-create-btn {
		background: var(--accent);
		color: white;
		border: none;
		width: 32px;
		height: 32px;
		border-radius: 50%;
		display: flex;
		align-items: center;
		justify-content: center;
		cursor: pointer;
		flex-shrink: 0;
	}

	.mobile-create-btn:active {
		opacity: 0.8;
	}

	.trash-notebook .note-title {
		display: flex;
		align-items: center;
		gap: 6px;
	}

	.trash-notebook-actions {
		display: flex;
		gap: 6px;
		margin-top: 6px;
	}

	.trash-nb-btn {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		padding: 3px 8px;
		border: none;
		border-radius: 4px;
		font-size: 0.75rem;
		cursor: pointer;
		background: var(--bg-hover);
		color: var(--text-secondary);
	}

	.trash-nb-btn:hover {
		background: var(--bg-active);
	}

	.trash-nb-btn.delete:hover {
		background: color-mix(in srgb, #ef4444 15%, transparent);
		color: #ef4444;
	}

	.trash-notebook.busy {
		opacity: 0.6;
		pointer-events: none;
	}

	.trash-busy {
		display: flex;
		align-items: center;
		gap: 6px;
	}

	.spinner {
		animation: spin 0.8s linear infinite;
	}

	@keyframes spin {
		to { transform: rotate(360deg); }
	}

	/* Calendar */
	.cal {
		padding: 8px 10px;
		border-bottom: 1px solid var(--border);
	}

	.cal-nav {
		display: flex;
		align-items: center;
		justify-content: space-between;
		margin-bottom: 6px;
	}

	.cal-nav-btn {
		background: none;
		border: none;
		cursor: pointer;
		color: var(--text-secondary);
		padding: 4px;
		border-radius: 4px;
		display: flex;
		align-items: center;
	}

	.cal-nav-btn:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.cal-title {
		font-size: 0.8rem;
		font-weight: 600;
		color: var(--text-primary);
		background: none;
		border: none;
		cursor: pointer;
		padding: 2px 8px;
		border-radius: 4px;
	}

	.cal-title:hover {
		background: var(--bg-hover);
	}

	.cal-grid {
		display: grid;
		grid-template-columns: repeat(7, 1fr);
		gap: 2px;
	}

	.cal-head {
		font-size: 0.65rem;
		color: var(--text-secondary);
		text-align: center;
		padding: 2px 0;
		font-weight: 500;
	}

	.cal-cell {
		position: relative;
		aspect-ratio: 1;
		display: flex;
		align-items: center;
		justify-content: center;
		font-size: 12px;
		border: 1px solid transparent;
		border-radius: 6px;
		background: none;
		color: var(--text-secondary);
		cursor: pointer;
		padding: 0;
	}

	.cal-cell.empty {
		cursor: default;
	}

	.cal-cell:not(.empty):hover {
		background: var(--bg-hover);
	}

	.cal-cell.today {
		border-color: var(--accent);
		color: var(--accent);
		font-weight: 700;
	}

	.cal-cell.has-note {
		background: color-mix(in srgb, var(--accent) 15%, transparent);
		color: var(--text-primary);
		font-weight: 600;
	}

	.cal-cell.has-note:hover {
		background: color-mix(in srgb, var(--accent) 25%, transparent);
	}

	.cal-cell.active {
		background: var(--accent);
		color: white;
		font-weight: 600;
	}

	.cal-cell.active:hover {
		background: var(--accent);
	}
</style>
