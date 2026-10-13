<script lang="ts">
	import { onDestroy, tick } from 'svelte';
	import { get } from 'svelte/store';
	import {
		notebooks,
		notes,
		tags,
		viewMode,
		activeNotebook,
		activeNotePath,
		editorDirty,
		activeNote,
		activeTag,
		showSearch,
		sidebarCollapsed,
		showSettings,
		showInfo,
		notebookIcons,
		tagStyles,
		appConfig,
		quickAccessPaths,
		collapsedNotebooks,
		rootNoteCount,
		notebookSortMode,
		notebookOrder,
		noteOrder
	} from '$lib/stores/app';
	import { getNotebooks, getAllTags, createNotebook, deleteNotebook, renameNotebook, moveNotebook, getNotebookIcons, setNotebookIcon, getTagStyles, importNotebookIcon, getQuickAccess, addQuickAccess, removeQuickAccess, emptyTrash, moveNote, countRootNotes } from '$lib/api';
	import { open as openDialog } from '@tauri-apps/plugin-dialog';
	import { readFile } from '@tauri-apps/plugin-fs';
	import { convertFileSrc } from '@tauri-apps/api/core';
	import type { NotebookEntry } from '$lib/types';
	import { isMobile } from '$lib/platform';
	import { decodeNoteDragPaths } from '$lib/utils/note-drag';
	import { notebookAppearance, tagAppearance } from '$lib/utils/appearance-mutations';
	import NotebookGlyph from './NotebookGlyph.svelte';
	import TagLabel from './TagLabel.svelte';
	import TagStylePicker from './TagStylePicker.svelte';
	import {
		NOTEBOOK_ICON_OPTIONS,
		COMMON_NOTEBOOK_ICON_COUNT,
		isCommonNotebookIcon,
		decodeNotebookIcon,
		encodeBuiltinNotebookIcon,
		normalizeNotebookIconKey,
		type NotebookIconId
	} from '$lib/utils/notebook-icons';
	import { TAG_COLOR_PRESETS } from '$lib/utils/tag-styles';

	let { onViewChanged = () => {}, onBeforeNoteChange = async () => true, onBeforePathMutation = async () => null, onPathsMoved = async (_old: string, _next: string) => {} }: {
		onViewChanged?: () => void;
		onBeforeNoteChange?: () => Promise<boolean>;
		onBeforePathMutation?: () => Promise<(() => void) | null>;
		onPathsMoved?: (oldPath: string, nextPath: string) => Promise<void>;
	} = $props();

	const modKey = navigator.platform.startsWith('Mac') ? '⌘' : 'Ctrl';
	const ownerVault = get(appConfig)?.active_vault ?? null;
	let disposed = false;
	let refreshRequest = 0;
	let tagRefreshRequest = 0;

	function ownerIsCurrent() {
		return !disposed && (get(appConfig)?.active_vault ?? null) === ownerVault;
	}

	onDestroy(() => {
		disposed = true;
		refreshRequest++;
		nbStopDragListeners();
	});

	// Whether any top nav item (All Notes, Quick Access, Tasks, Daily Notes, Trash) is visible.
	// When all are hidden we drop the empty <nav> and its divider so the tree sits flush.
	const anyNavItem = $derived(
		$appConfig?.show_all_notes !== false ||
		$appConfig?.show_quick_access !== false ||
		$appConfig?.show_tasks !== false ||
		$appConfig?.show_daily_notes !== false ||
		$appConfig?.show_trash !== false
	);

	let editingNotebook = $state<string | null>(null);
	let editValue = $state('');
	let renameInput: HTMLInputElement | null = $state(null);
	let newNotebookName = $state('');
	let showNewNotebook = $state(false);
	let iconError = $state('');
	let newNotebookInput: HTMLInputElement | null = $state(null);
	let dropTargetPath = $state<string | null>(null);
	let dropPosition = $state<'above' | 'into' | 'below' | null>(null);
	let draggedNotebookPath = $state<string | null>(null);
	let nbPointer: { src: string } | null = null;

	// Notebook paths arrive from Rust with native separators (backslashes on Windows), so path math must be separator-agnostic.
	const norm = (p: string) => (p ?? '').replace(/\\/g, '/');
	const lastSep = (p: string) => Math.max(p.lastIndexOf('/'), p.lastIndexOf('\\'));
	const parentOf = (p: string) => { const i = lastSep(p); return i < 0 ? '' : p.slice(0, i); };
	const baseOf = (p: string) => p.slice(lastSep(p) + 1);
	const joinPath = (parent: string, name: string) => parent + (parent.includes('\\') ? '\\' : '/') + name;
	const isDescendant = (child: string, anc: string) => child.startsWith(anc + '/') || child.startsWith(anc + '\\');

	function migratePaths(oldPath: string, newPath: string) {
		const replace = (path: string) => path === oldPath ? newPath
			: isDescendant(path, oldPath) ? newPath + path.slice(oldPath.length) : path;
		$collapsedNotebooks = $collapsedNotebooks.map(replace);
		$notebookOrder = Object.fromEntries(Object.entries($notebookOrder).map(([path, index]) => [replace(path), index]));
		$noteOrder = Object.fromEntries(Object.entries($noteOrder).map(([path, index]) => [replace(path), index]));
		if ($activeNotebook && replace($activeNotebook.path) !== $activeNotebook.path) {
			const path = replace($activeNotebook.path);
			$activeNotebook = {
				...$activeNotebook, path,
				relative_path: path.slice(ownerVault!.length + 1).replace(/\\/g, '/'),
				name: baseOf(path)
			};
		}
		$notes = $notes.map(note => {
			const path = replace(note.path);
			return path === note.path ? note : { ...note, path, relative_path: path.slice(ownerVault!.length + 1).replace(/\\/g, '/') };
		});
	}

	// Apply manual ordering recursively when in 'manual' sort mode.
	// In 'alphabetical' mode the Rust scanner already sorts, so we pass through unchanged.
	function sortNotebooksTree(tree: NotebookEntry[]): NotebookEntry[] {
		if ($notebookSortMode !== 'manual') return tree;
		const order = $notebookOrder;
		const sorted = [...tree].sort((a, b) => {
			const oa = order[a.path] ?? Number.MAX_SAFE_INTEGER;
			const ob = order[b.path] ?? Number.MAX_SAFE_INTEGER;
			if (oa !== ob) return oa - ob;
			return a.name.toLowerCase().localeCompare(b.name.toLowerCase());
		});
		return sorted.map(nb => ({ ...nb, children: sortNotebooksTree(nb.children) }));
	}
	let sortedNotebooks = $derived(sortNotebooksTree($notebooks));
	let contextMenu = $state<{ x: number; y: number; notebook: NotebookEntry } | null>(null);
	let iconPickerNotebook = $state<NotebookEntry | null>(null);
	let iconPickerElement = $state<HTMLDivElement | null>(null);
	let iconPickerColor = $state<string | null>(null);
	let iconPickerShowAll = $state<boolean | null>(null);
	const iconPickerActive = $derived(iconPickerNotebook
		? decodeNotebookIcon($notebookIcons[normalizeNotebookIconKey(iconPickerNotebook.relative_path)])?.icon ?? null
		: null);
	// Starts open when the current icon is one of the extra icons.
	const iconPickerExpanded = $derived(iconPickerShowAll ?? !isCommonNotebookIcon(iconPickerActive));
	let trashContextMenu = $state<{ x: number; y: number } | null>(null);
	let tagsCollapsed = $state(true);
	let tagContextMenu = $state<{ x: number; y: number; tag: string } | null>(null);
	let stylePickerTag = $state<string | null>(null);
	let deleteConfirm = $state<NotebookEntry | null>(null);
	function toggleCollapse(nb: NotebookEntry, e: Event) {
		e.stopPropagation();
		if ($collapsedNotebooks.includes(nb.path)) {
			$collapsedNotebooks = $collapsedNotebooks.filter(p => p !== nb.path);
		} else {
			$collapsedNotebooks = [...$collapsedNotebooks, nb.path];
		}
	}
	// Collapse-all / expand-all: toggles every notebook that has children.
	function collectParentPaths(tree: NotebookEntry[]): string[] {
		const out: string[] = [];
		for (const nb of tree) {
			if (nb.children.length > 0) {
				out.push(nb.path);
				out.push(...collectParentPaths(nb.children));
			}
		}
		return out;
	}
	const allParentPaths = $derived(collectParentPaths($notebooks));
	const allCollapsed = $derived(allParentPaths.length > 0 && allParentPaths.every((p) => $collapsedNotebooks.includes(p)));
	function toggleAllCollapse() {
		$collapsedNotebooks = allCollapsed ? [] : [...allParentPaths];
	}


	export async function refreshTags() {
		if (!ownerIsCurrent()) return;
		const request = ++tagRefreshRequest;
		try {
			const nextTags = await getAllTags();
			if (ownerIsCurrent() && request === tagRefreshRequest) $tags = nextTags;
		} catch (error) {
			console.error('Failed to refresh tags:', error);
		}
	}

	export async function refresh() {
		if (!ownerIsCurrent()) return;
		const request = ++refreshRequest;
		const tagRequest = isMobile ? tagRefreshRequest : ++tagRefreshRequest;
		const iconRevision = notebookAppearance.snapshot(ownerVault!);
		const styleRevision = tagAppearance.snapshot(ownerVault!);
		try {
			// Mobile derives tags from notes instead of issuing another full-vault scan.
			const [nbs, appearances, qaNotes, rootCount, nextTags] = await Promise.all([
				getNotebooks(),
				Promise.allSettled([getNotebookIcons(ownerVault!), getTagStyles(ownerVault!)]),
				getQuickAccess(ownerVault!),
				countRootNotes(),
				isMobile ? Promise.resolve(null) : getAllTags(),
			]);
			if (!ownerIsCurrent() || request !== refreshRequest) return;
			$notebooks = nbs;
			const [icons, styles] = appearances;
			if (notebookAppearance.acceptsSnapshot(ownerVault!, iconRevision)) {
				if (icons.status === 'fulfilled') $notebookIcons = icons.value;
				else iconError = `Could not read notebook icons: ${String(icons.reason)}`;
			}
			if (tagAppearance.acceptsSnapshot(ownerVault!, styleRevision)) {
				if (styles.status === 'fulfilled') $tagStyles = styles.value;
				else iconError = `Could not read tag appearances: ${String(styles.reason)}`;
			}
			$quickAccessPaths = qaNotes.map(n => n.relative_path);
			$rootNoteCount = rootCount;
			if (nextTags && tagRequest === tagRefreshRequest) $tags = nextTags;
		} catch (e) {
			console.error('Failed to refresh sidebar:', e);
		}
	}

	function selectAllNotes() {
		$viewMode = 'all';
		$activeNotebook = null;
		$activeTag = null;
		onViewChanged();
	}

	function selectUnfiled() {
		const vault = $appConfig?.active_vault;
		if (!vault) return;
		if ($viewMode === 'notebook' && $activeNotebook?.relative_path === '') {
			if (isMobile) {
				onViewChanged();
				return;
			}
			selectAllNotes();
			return;
		}
		$viewMode = 'notebook';
		$activeNotebook = { name: 'Unfiled Notes', path: vault, relative_path: '', children: [], note_count: $rootNoteCount };
		$activeTag = null;
		onViewChanged();
	}

	async function selectNotebook(nb: NotebookEntry) {
		// Deselect if clicking the already-active notebook; expand/collapse is the chevron's job
		if ($viewMode === 'notebook' && $activeNotebook?.path === nb.path) {
			if (isMobile) {
				onViewChanged();
				return;
			}
			selectAllNotes();
			return;
		}
		$viewMode = 'notebook';
		$activeNotebook = nb;
		$activeTag = null;
		onViewChanged();
	}

	function selectTag(tag: string) {
		$viewMode = 'tag';
		$activeTag = tag;
		$activeNotebook = null;
		onViewChanged();
	}

	function selectQuickAccess() {
		$viewMode = 'quickaccess';
		$activeNotebook = null;
		$activeTag = null;
		onViewChanged();
	}

	function selectDaily() {
		$viewMode = 'daily';
		$activeNotebook = null;
		$activeTag = null;
		onViewChanged();
	}

	function selectTasks() {
		$viewMode = 'tasks';
		$activeNotebook = null;
		$activeTag = null;
		onViewChanged();
	}

	function selectTrash() {
		$viewMode = 'trash';
		$activeNotebook = null;
		$activeTag = null;
		onViewChanged();
	}

	let newNotebookParent = $state<NotebookEntry | null>(null);

	function getNotebookNameSegments(name: string): string[] | null {
		const segments = name.split('/').map((segment) => segment.trim());
		if (segments.some((segment) => !segment || segment === '.' || segment === '..' || segment.includes('\\'))) {
			return null;
		}
		return segments;
	}

	async function focusNewNotebookInput() {
		await tick();
		if (!ownerIsCurrent()) return;
		newNotebookInput?.focus();
	}

	async function toggleNewNotebookInput() {
		if (showNewNotebook) {
			showNewNotebook = false;
			newNotebookName = '';
			newNotebookParent = null;
			return;
		}

		// Unfiled Notes is the root pseudo-notebook (empty relative_path); treat it as root.
		newNotebookParent = ($viewMode === 'notebook' && $activeNotebook?.relative_path) ? $activeNotebook : null;
		showNewNotebook = true;
		await focusNewNotebookInput();
	}

	async function startNewNotebookFromSidebar(e: MouseEvent) {
		const target = e.target as HTMLElement | null;
		if (target?.closest('button, input, .context-menu, .delete-confirm-overlay')) return;

		newNotebookParent = ($viewMode === 'notebook' && $activeNotebook?.relative_path) ? $activeNotebook : null;
		newNotebookName = '';
		showNewNotebook = true;
		await focusNewNotebookInput();
	}

	async function handleCreateNotebook() {
		if (!newNotebookName.trim()) return;
		try {
			const trimmedName = newNotebookName.trim();
			const parentRel = newNotebookParent?.relative_path ?? null;
			const name = parentRel ? trimmedName : getNotebookNameSegments(trimmedName)?.join('/');
			if (!name) return;

			await createNotebook(ownerVault!, parentRel, name);
			if (!ownerIsCurrent()) return;
			newNotebookName = '';
			showNewNotebook = false;
			newNotebookParent = null;
			await refresh();
		} catch (e) {
			console.error('Failed to create notebook:', e);
		}
	}

	function handleNewNotebookKeydown(e: KeyboardEvent) {
		if (e.key === 'Enter') {
			handleCreateNotebook();
			return;
		}
		if (e.key === 'Escape') {
			showNewNotebook = false;
			newNotebookName = '';
			newNotebookParent = null;
			return;
		}
		if (e.key === 'Backspace' && newNotebookParent && newNotebookName.length === 0) {
			e.preventDefault();
			newNotebookParent = null;
		}
	}

	async function startNewSubNotebook(nb: NotebookEntry) {
		contextMenu = null;
		newNotebookParent = nb;
		newNotebookName = '';
		showNewNotebook = true;
		// Expand parent so the new child is visible
		if ($collapsedNotebooks.includes(nb.path)) {
			$collapsedNotebooks = $collapsedNotebooks.filter(p => p !== nb.path);
		}
		await focusNewNotebookInput();
	}

	async function handleRename(nb: NotebookEntry) {
		if (!editValue.trim() || editValue.trim() === nb.name) {
			editingNotebook = null;
			return;
		}
		const release = await onBeforePathMutation();
		if (!release) return;
		try {
			if (!ownerIsCurrent()) return;
			const newPath = await renameNotebook(ownerVault!, nb.path, editValue.trim());
			if (!ownerIsCurrent()) return;
			migratePaths(nb.path, newPath);
			await onPathsMoved(nb.path, newPath);
			if (!ownerIsCurrent()) return;
			editingNotebook = null;
			await refresh();
		} catch (e) {
			console.error('Failed to rename:', e);
		} finally {
			release();
		}
	}

	function countNotesRecursive(nb: NotebookEntry): number {
		return nb.note_count + nb.children.reduce((sum, c) => sum + countNotesRecursive(c), 0);
	}

	function handleDelete(nb: NotebookEntry) {
		contextMenu = null;
		const total = countNotesRecursive(nb);
		if (total > 0) {
			deleteConfirm = nb;
		} else {
			confirmDelete(nb);
		}
	}

	function dismissDeleteConfirm(event: MouseEvent) {
		if (event.target === event.currentTarget) deleteConfirm = null;
	}

	async function confirmDelete(nb: NotebookEntry) {
		deleteConfirm = null;
		try {
			if (!(await onBeforeNoteChange())) return;
			if (!ownerIsCurrent()) return;
			await deleteNotebook(ownerVault!, nb.path);
			if (!ownerIsCurrent()) return;
			if ($activeNotebook?.path === nb.path) {
				selectAllNotes();
			}
			await refresh();
		} catch (e) {
			console.error('Failed to delete:', e);
		}
	}

	async function handleNoteDrop(e: DragEvent, nb: NotebookEntry) {
		e.preventDefault();
		dropTargetPath = null;
		const payload = e.dataTransfer?.getData('text/plain') ?? '';
		const notePaths = [...new Set(decodeNoteDragPaths(payload))]
			.filter((path) => norm(parentOf(path)) !== norm(nb.path));
		if (notePaths.length === 0) return;
		const release = await onBeforePathMutation();
		if (!release) return;
		try {
			if (!ownerIsCurrent()) return;
			for (const notePath of notePaths) {
				try {
					const newPath = await moveNote(ownerVault!, notePath, nb.path);
					if (!ownerIsCurrent()) return;
					migratePaths(notePath, newPath);
					await onPathsMoved(notePath, newPath);
				} catch (error) {
					console.error('Failed to move note:', notePath, error);
				}
			}
			if (ownerIsCurrent()) await refresh();
		} finally {
			release();
		}
	}

	async function handleNotebookDrop(e: DragEvent, destPath: string) {
		e.preventDefault();
		const srcPath = e.dataTransfer?.getData('text/plain');
		if (srcPath) await nestNotebook(srcPath, destPath);
	}

	async function nestNotebook(srcPath: string, destPath: string) {
		dropTargetPath = null;
		dropPosition = null;
		draggedNotebookPath = null;
		// Don't move onto self or descendant
		if (destPath === srcPath || isDescendant(destPath, srcPath)) return;
		// Don't move if already in that parent
		const parentDir = parentOf(srcPath);
		if (parentDir === destPath) return;
		const release = await onBeforePathMutation();
		if (!release) return;
		try {
			if (!ownerIsCurrent()) return;
			const newPath = await moveNotebook(ownerVault!, srcPath, destPath);
			if (!ownerIsCurrent()) return;
			migratePaths(srcPath, newPath);
			await onPathsMoved(srcPath, newPath);
			if (ownerIsCurrent()) await refresh();
		} catch (error) {
			console.error('Failed to move notebook:', error);
		} finally {
			release();
		}
	}


	function nbStopDragListeners() {
		window.removeEventListener('pointermove', nbPointerMove);
		window.removeEventListener('pointerup', nbDragEnd);
		window.removeEventListener('pointercancel', nbDragEnd);
		window.removeEventListener('mouseup', nbDragEnd);
	}

	function nbHandleDown(e: PointerEvent, nb: NotebookEntry) {
		if (e.pointerType === 'mouse' && e.button !== 0) return;
		e.stopPropagation();
		e.preventDefault();
		nbStopDragListeners();
		nbPointer = { src: nb.path };
		draggedNotebookPath = nb.path;
		// Capture helps touch hold the gesture; on Windows mouse it makes WebView2 swallow the release, so skip it there.
		if (e.pointerType === 'touch') {
			try { (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId); } catch {}
		}
		window.addEventListener('pointermove', nbPointerMove, { passive: false });
		window.addEventListener('pointerup', nbDragEnd);
		window.addEventListener('pointercancel', nbDragEnd);
		window.addEventListener('mouseup', nbDragEnd);
	}

	function nbPointerMove(e: PointerEvent) {
		if (!nbPointer) return;
		e.preventDefault();
		const el = document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null;
		if (el?.closest('[data-nb-root]')) { dropTargetPath = '__root__'; dropPosition = null; return; }
		const row = el?.closest('[data-nb-path]') as HTMLElement | null;
		const path = row?.getAttribute('data-nb-path');
		if (!path) { dropTargetPath = null; dropPosition = null; return; }
		const src = nbPointer.src;
		if (path === src || path.startsWith(src + '/')) { dropTargetPath = null; dropPosition = null; return; }
		const rect = row!.getBoundingClientRect();
		const relY = (e.clientY - rect.top) / rect.height;
		const pos: 'above' | 'into' | 'below' = relY < 0.3 ? 'above' : relY > 0.7 ? 'below' : 'into';
		if (pos === 'into' && src.substring(0, src.lastIndexOf('/')) === path) { dropTargetPath = null; dropPosition = null; return; }
		dropTargetPath = path;
		dropPosition = pos;
	}

	function nbDragEnd() {
		if (!nbPointer) return;
		nbStopDragListeners();
		const src = nbPointer.src;
		const target = dropTargetPath;
		const pos = dropPosition;
		nbPointer = null;
		draggedNotebookPath = null;
		dropTargetPath = null;
		dropPosition = null;
		if (!target) return;
		if (target === '__root__') {
			const vaultRoot = $appConfig?.active_vault;
			if (vaultRoot) nestNotebook(src, vaultRoot);
		} else if (pos === 'above' || pos === 'below') {
			handleNotebookReorder(src, target, pos);
		} else if (pos === 'into') {
			nestNotebook(src, target);
		}
	}

	function findSiblingsOfPath(tree: NotebookEntry[], parentPath: string, vaultRoot: string): NotebookEntry[] {
		const target = norm(parentPath);
		if (target === norm(vaultRoot)) return tree;
		for (const nb of tree) {
			if (norm(nb.path) === target) return nb.children;
			if (nb.children.length > 0) {
				const found = findSiblingsOfPath(nb.children, parentPath, vaultRoot);
				if (found.length > 0) return found;
			}
		}
		return [];
	}

	async function handleNotebookReorder(srcPath: string, targetPath: string, position: 'above' | 'below') {
		dropTargetPath = null;
		dropPosition = null;
		draggedNotebookPath = null;
		if (srcPath === targetPath) return;
		if (isDescendant(targetPath, srcPath)) return; // can't move into descendant
		const vaultRoot = $appConfig?.active_vault;
		if (!vaultRoot) return;

		const srcParent = parentOf(srcPath);
		const targetParent = parentOf(targetPath);
		let actualSrcPath = srcPath;
		const release = srcParent !== targetParent ? await onBeforePathMutation() : () => {};
		if (!release) return;
		try {
			// Step 1: if different parents, move src into target's parent first
			if (srcParent !== targetParent) {
				if (!ownerIsCurrent()) return;
				const newPath = await moveNotebook(ownerVault!, srcPath, targetParent);
				if (!ownerIsCurrent()) return;
				migratePaths(srcPath, newPath);
				await onPathsMoved(srcPath, newPath);
				await refresh();
				if (!ownerIsCurrent()) return;
				actualSrcPath = newPath;
			}

			// Step 2: reorder within target's parent
			const siblings = findSiblingsOfPath($notebooks, targetParent, vaultRoot);
			const sortedSiblings = sortNotebooksTree(siblings);
			const without = sortedSiblings.filter(s => s.path !== actualSrcPath);
			const targetIdx = without.findIndex(s => s.path === targetPath);
			if (targetIdx === -1) return;
			const insertIdx = position === 'above' ? targetIdx : targetIdx + 1;
			const newSequence = [
				...without.slice(0, insertIdx).map(s => s.path),
				actualSrcPath,
				...without.slice(insertIdx).map(s => s.path),
			];

			// Assign sequential positions to all siblings (only within this parent)
			const newOrder = { ...$notebookOrder };
			newSequence.forEach((path, idx) => {
				newOrder[path] = idx;
			});
			$notebookOrder = newOrder;

			// Auto-enable manual mode (the act of reordering implies user wants this)
			if ($notebookSortMode !== 'manual') $notebookSortMode = 'manual';
		} catch (e) {
			console.error('Failed to reorder notebook:', e);
		} finally {
			release();
		}
	}

	async function startRename(nb: NotebookEntry) {
		contextMenu = null;
		editingNotebook = nb.path;
		editValue = nb.name;
		await tick();
		if (!ownerIsCurrent()) return;
		renameInput?.focus();
		renameInput?.select();
	}

	async function openIconPicker(nb: NotebookEntry) {
		contextMenu = null;
		iconPickerColor = decodeNotebookIcon(get(notebookIcons)[normalizeNotebookIconKey(nb.relative_path)])?.color ?? null;
		iconPickerShowAll = null;
		iconPickerNotebook = nb;
		await tick();
		if (!ownerIsCurrent()) return;
		iconPickerElement?.focus();
	}

	function publishNotebookIcon(key: string, value: string | null) {
		if (get(appConfig)?.active_vault !== ownerVault) return;
		const icons = { ...get(notebookIcons) };
		if (value === null) delete icons[key];
		else icons[key] = value;
		notebookIcons.set(icons);
	}

	async function handleBuiltinIcon(nb: NotebookEntry, icon: NotebookIconId | null, close = true) {
		if (!ownerVault || !ownerIsCurrent()) return;
		const key = normalizeNotebookIconKey(nb.relative_path);
		const latest = notebookAppearance.choose(ownerVault, key);
		const value = icon || iconPickerColor ? encodeBuiltinNotebookIcon(icon, iconPickerColor) : null;
		iconError = '';
		try {
			await notebookAppearance.run(ownerVault, key, latest,
				() => setNotebookIcon(ownerVault, key, value),
				() => publishNotebookIcon(key, value));
			if (close && ownerIsCurrent() && latest()) iconPickerNotebook = null;
		} catch (e) {
			if (ownerIsCurrent() && latest()) iconError = String(e);
		}
	}

	// A custom image has no color; the choice is kept for the next built-in icon.
	function handleIconColor(nb: NotebookEntry, color: string | null) {
		iconPickerColor = color;
		const stored = get(notebookIcons)[normalizeNotebookIconKey(nb.relative_path)];
		const current = decodeNotebookIcon(stored);
		if (current || !stored) void handleBuiltinIcon(nb, current?.icon ?? null, false);
	}

	async function handleCustomIcon(nb: NotebookEntry) {
		if (!ownerVault || !ownerIsCurrent()) return;
		const key = normalizeNotebookIconKey(nb.relative_path);
		const latest = notebookAppearance.choose(ownerVault, key);
		iconPickerNotebook = null;
		iconError = '';
		try {
			const selected = await openDialog({
				multiple: false,
				filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'gif', 'svg', 'webp', 'ico'] }]
			});
			if (!selected || !ownerIsCurrent() || !latest()) return;
			const filePath = selected as string;
			const data = await readFile(filePath);
			if (!ownerIsCurrent() || !latest()) return;
			const fileName = baseOf(filePath) || 'icon.png';
			await notebookAppearance.run(ownerVault, key, latest,
				() => importNotebookIcon(ownerVault, key, `notebook-icon-${fileName}`, Array.from(data)),
				(value) => publishNotebookIcon(key, value));
		} catch (e) {
			if (ownerIsCurrent() && latest()) iconError = String(e);
		}
	}

	async function handleRemoveIcon(nb: NotebookEntry) {
		if (!ownerVault || !ownerIsCurrent()) return;
		const key = normalizeNotebookIconKey(nb.relative_path);
		const latest = notebookAppearance.choose(ownerVault, key);
		contextMenu = null;
		iconPickerNotebook = null;
		iconError = '';
		try {
			await notebookAppearance.run(ownerVault, key, latest,
				() => setNotebookIcon(ownerVault, key, null),
				() => publishNotebookIcon(key, null));
		} catch (e) {
			if (ownerIsCurrent() && latest()) iconError = String(e);
		}
	}

	function getNotebookIconSrc(nb: NotebookEntry): string | null {
		const iconPath = $notebookIcons[normalizeNotebookIconKey(nb.relative_path)];
		if (!iconPath || iconPath.startsWith('builtin:')) return null;
		const vaultRoot = $appConfig?.active_vault;
		if (!vaultRoot) return null;
		return convertFileSrc(`${vaultRoot}/${iconPath}`);
	}

	function clampMenu(x: number, y: number, menuWidth = 220, menuHeight = 300): { x: number; y: number } {
		if (x + menuWidth > window.innerWidth) x = window.innerWidth - menuWidth - 8;
		if (y + menuHeight > window.innerHeight) y = window.innerHeight - menuHeight - 8;
		if (x < 4) x = 4;
		if (y < 4) y = 4;
		return { x, y };
	}

	function onContextMenu(e: MouseEvent, nb: NotebookEntry) {
		e.preventDefault();
		const { x, y } = clampMenu(e.clientX, e.clientY);
		contextMenu = { x, y, notebook: nb };
	}

	function openNotebookMenu(e: Event, nb: NotebookEntry) {
		e.preventDefault();
		e.stopPropagation();
		const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
		const { x, y } = clampMenu(rect.right - 200, rect.bottom + 4, 200, 280);
		contextMenu = { x, y, notebook: nb };
	}

	function closeContextMenu() {
		contextMenu = null;
	}

	function onTagContextMenu(e: MouseEvent, tag: string) {
		e.preventDefault();
		e.stopPropagation();
		const { x, y } = clampMenu(e.clientX, e.clientY, 200, 80);
		tagContextMenu = { x, y, tag };
	}

	function openTagMenu(e: Event, tag: string) {
		e.preventDefault();
		e.stopPropagation();
		const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
		const { x, y } = clampMenu(rect.right - 200, rect.bottom + 4, 200, 80);
		tagContextMenu = { x, y, tag };
	}

	function openTagStylePicker(tag: string) {
		tagContextMenu = null;
		stylePickerTag = tag;
	}

	function onTrashContextMenu(e: MouseEvent) {
		e.preventDefault();
		e.stopPropagation();
		const { x, y } = clampMenu(e.clientX, e.clientY, 180, 60);
		trashContextMenu = { x, y };
	}

	async function handleEmptyTrash() {
		trashContextMenu = null;
		const previous = $activeNote;
		const path = $activeNotePath;
		try {
			await emptyTrash(ownerVault!);
			if (!ownerIsCurrent()) return;
			// Clear selection and refresh the list (with cache invalidation)
			if ($activeNotePath === path && $activeNote === previous) {
				$activeNote = null;
				$activeNotePath = '';
			}
			onViewChanged();
		} catch (e) {
			console.error('Failed to empty trash:', e);
		}
	}


	// Close context menu on click outside
	function handleWindowClick() {
		if (contextMenu) contextMenu = null;
		if (trashContextMenu) trashContextMenu = null;
		if (tagContextMenu) tagContextMenu = null;
	}
</script>

<svelte:window onclick={handleWindowClick} onkeydown={(e) => { if (e.key === 'Escape') { iconPickerNotebook = null; tagContextMenu = null; } }} />

<aside class="sidebar" class:collapsed={!isMobile && $sidebarCollapsed} class:mobile={isMobile} class:nav-empty={!anyNavItem}>
	{#if !isMobile}
	<div class="sidebar-header">
		<button class="collapse-btn" onclick={() => ($sidebarCollapsed = !$sidebarCollapsed)} title="Toggle sidebar">
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
				{#if $sidebarCollapsed}
					<polyline points="9 6 15 12 9 18" />
				{:else}
					<polyline points="15 6 9 12 15 18" />
				{/if}
			</svg>
		</button>
			<button class="icon-btn" onclick={() => ($showSearch = true)} title={`Search (${modKey}+F)`}>
				<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<circle cx="11" cy="11" r="8" />
					<line x1="21" y1="21" x2="16.65" y2="16.65" />
				</svg>
			</button>
	</div>
	{/if}

	{#if isMobile || !$sidebarCollapsed}
		{#if anyNavItem}
		<nav class="sidebar-nav">
			{#if $appConfig?.show_all_notes !== false}
			<button
				class="nav-item"
				class:active={$viewMode === 'all'}
				onclick={selectAllNotes}
			>
				<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<path d="M16 3H5a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2V8z" />
					<polyline points="14 3 14 8 21 8" />
				</svg>
				<span>All Notes</span>
			</button>
			{/if}

			{#if $appConfig?.show_quick_access !== false}
			<button
				class="nav-item"
				class:active={$viewMode === 'quickaccess'}
				onclick={selectQuickAccess}
			>
				<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" />
				</svg>
				<span>Quick Access</span>
			</button>
			{/if}

			{#if $appConfig?.show_tasks !== false}
			<button
				class="nav-item"
				class:active={$viewMode === 'tasks'}
				onclick={selectTasks}
			>
				<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<path d="M9 11l3 3L22 4" />
					<path d="M21 12v7a2 2 0 01-2 2H5a2 2 0 01-2-2V5a2 2 0 012-2h11" />
				</svg>
				<span>Tasks</span>
			</button>
			{/if}

			{#if $appConfig?.show_daily_notes !== false}
			<button
				class="nav-item"
				class:active={$viewMode === 'daily'}
				onclick={selectDaily}
			>
				<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<rect x="3" y="4" width="18" height="18" rx="2" ry="2" />
					<line x1="16" y1="2" x2="16" y2="6" />
					<line x1="8" y1="2" x2="8" y2="6" />
					<line x1="3" y1="10" x2="21" y2="10" />
				</svg>
				<span>Daily Notes</span>
			</button>
			{/if}

			{#if $appConfig?.show_trash !== false}
			<button
				class="nav-item"
				class:active={$viewMode === 'trash'}
				onclick={selectTrash}
				oncontextmenu={onTrashContextMenu}
			>
				<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<polyline points="3 6 5 6 21 6" /><path d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2" /><line x1="10" y1="11" x2="10" y2="17" /><line x1="14" y1="11" x2="14" y2="17" />
				</svg>
				<span>Trash</span>
			</button>
			{/if}
		</nav>
		{/if}

		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div class="section" ondblclick={startNewNotebookFromSidebar}>
			<!-- svelte-ignore a11y_no_static_element_interactions -->
			<div
				class="section-header"
				class:drop-target={dropTargetPath === '__root__'}
				data-nb-root
				ondragover={(e) => {
					if (draggedNotebookPath) {
						e.preventDefault();
						e.dataTransfer!.dropEffect = 'move';
						dropTargetPath = '__root__';
					}
				}}
				ondragleave={() => { if (dropTargetPath === '__root__') dropTargetPath = null; }}
				ondrop={(e) => {
					if (draggedNotebookPath) {
						const vaultRoot = $appConfig?.active_vault;
						if (vaultRoot) handleNotebookDrop(e, vaultRoot);
					}
				}}
			>
				<span class="section-title">Notebooks</span>
				<div class="section-actions">
					{#if allParentPaths.length > 0}
						<button class="icon-btn-sm" onclick={(e) => { e.stopPropagation(); toggleAllCollapse(); }} title={allCollapsed ? 'Expand all notebooks' : 'Collapse all notebooks'} aria-label={allCollapsed ? 'Expand all notebooks' : 'Collapse all notebooks'}>
							<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
								{#if allCollapsed}
									<path d="m7 15 5 5 5-5" /><path d="m7 9 5-5 5 5" />
								{:else}
									<path d="m7 20 5-5 5 5" /><path d="m7 4 5 5 5-5" />
								{/if}
							</svg>
						</button>
					{/if}
					<button class="icon-btn-sm" onclick={toggleNewNotebookInput} title={$viewMode === 'notebook' && $activeNotebook?.relative_path ? `New notebook inside ${$activeNotebook.name}` : 'New notebook'}>
						<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
							<line x1="12" y1="5" x2="12" y2="19" /><line x1="5" y1="12" x2="19" y2="12" />
						</svg>
					</button>
				</div>
			</div>

			{#if showNewNotebook}
				<div class="new-notebook-input">
					{#if newNotebookParent}
						<button class="new-notebook-parent" title="Click to create a top-level notebook instead" onmousedown={(e) => { e.preventDefault(); newNotebookParent = null; }}>
							{newNotebookParent.name}/<span class="new-notebook-parent-x" aria-hidden="true">×</span>
						</button>
					{/if}
					<input
						bind:this={newNotebookInput}
						type="text"
						bind:value={newNotebookName}
						placeholder={newNotebookParent ? 'Sub-notebook name...' : 'Notebook name...'}
						onkeydown={handleNewNotebookKeydown}
					/>
				</div>
			{/if}

			{#if iconError}<div class="sidebar-error" role="alert">{iconError}</div>{/if}
			<div class="notebook-list">
				{#if $rootNoteCount > 0}
					<button
						class="notebook-item"
						class:active={$viewMode === 'notebook' && $activeNotebook?.relative_path === ''}
						onclick={selectUnfiled}
						style="padding-left: 4px"
					>
					<span style="width:14px;flex-shrink:0"></span>
					<span class="notebook-icon-slot">
					<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="flex-shrink:0;opacity:0.6">
						<path d="M22 12h-6l-2 3h-4l-2-3H2" />
						<path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z" />
					</svg>
					</span>
					<span class="notebook-name">Unfiled Notes <span class="notebook-count">{$rootNoteCount}</span></span>
					</button>
				{/if}
				{#each sortedNotebooks as nb (nb.path)}
					{@render notebookItem(nb, 0)}
				{/each}
			</div>
		{#if isMobile && $tags.length > 0}
				<div class="tags-section-inline">
					<button class="section-header" onclick={() => tagsCollapsed = !tagsCollapsed}>
						<span class="section-title">Tags</span>
						<span class="tag-count" style="margin-left: auto">{$tags.length}</span>
					</button>
					{#if !tagsCollapsed}
						<div class="tag-list">
							{#each $tags as [tag, count] (tag)}
								{@render tagRow(tag, count)}
							{/each}
						</div>
					{/if}
				</div>
			{/if}
		</div>

		{#if !isMobile && $tags.length > 0}
			<div class="tags-section">
				<button class="section-header" onclick={() => tagsCollapsed = !tagsCollapsed}>
					<span class="section-title">Tags</span>
					<span class="tag-count" style="margin-left: auto">{$tags.length}</span>
				</button>
				{#if !tagsCollapsed}
					<div class="tag-list">
						{#each $tags as [tag, count] (tag)}
							{@render tagRow(tag, count)}
						{/each}
					</div>
				{/if}
			</div>
		{/if}
	{/if}

		<div class="sidebar-footer">
			<button class="icon-btn" onclick={() => ($showInfo = true)} title="Info">
				<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<circle cx="12" cy="12" r="10" />
					<line x1="12" y1="16" x2="12" y2="12" />
					<line x1="12" y1="8" x2="12.01" y2="8" />
				</svg>
			</button>
			<div class="sidebar-footer-right">
			<button class="icon-btn" onclick={() => ($showSettings = true)} title="Settings">
				<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<circle cx="12" cy="12" r="3" />
					<path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 010 2.83 2 2 0 01-2.83 0l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 012.83-2.83l.06.06A1.65 1.65 0 009 4.68a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z" />
				</svg>
			</button>

			</div>
		</div>
</aside>

{#if contextMenu}
	{#if isMobile}
		<button type="button" class="context-menu-backdrop" aria-label="Close notebook actions" onclick={closeContextMenu}></button>
	{/if}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="context-menu" class:mobile={isMobile} style="left: {contextMenu.x}px; top: {contextMenu.y}px" onmousedown={(e) => e.stopPropagation()}>
		<button onclick={() => startNewSubNotebook(contextMenu!.notebook)}>
			<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z" /><line x1="12" y1="11" x2="12" y2="17" /><line x1="9" y1="14" x2="15" y2="14" /></svg>
			New Sub-notebook
		</button>
		<button onclick={() => startRename(contextMenu!.notebook)}>
			<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7" /><path d="M18.5 2.5a2.121 2.121 0 013 3L12 15l-4 1 1-4 9.5-9.5z" /></svg>
			Rename
		</button>
		<button onclick={() => openIconPicker(contextMenu!.notebook)}>
			<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10" /><path d="M8 14s1.5 2 4 2 4-2 4-2" /><line x1="9" y1="9" x2="9.01" y2="9" /><line x1="15" y1="9" x2="15.01" y2="9" /></svg>
			{$notebookIcons[normalizeNotebookIconKey(contextMenu.notebook.relative_path)] ? 'Change Icon...' : 'Set Icon...'}
		</button>
		<button class="danger" onclick={() => handleDelete(contextMenu!.notebook)}>
			<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6" /><path d="M19 6l-1 14a2 2 0 01-2 2H8a2 2 0 01-2-2L5 6" /><path d="M10 11v6" /><path d="M14 11v6" /><path d="M9 6V4a1 1 0 011-1h4a1 1 0 011 1v2" /></svg>
			Delete
		</button>
	</div>
{/if}

{#if iconPickerNotebook}
	<div class="icon-picker-overlay" class:mobile={isMobile}>
		<button class="icon-picker-backdrop" aria-label="Close icon picker" onclick={() => iconPickerNotebook = null}></button>
		<div bind:this={iconPickerElement} class="icon-picker" role="dialog" aria-modal="true" aria-label={`Choose an icon for ${iconPickerNotebook.name}`} tabindex="-1">
			<header class="icon-picker-header">
				<div>
					<h3>Notebook icon</h3>
					<p>{iconPickerNotebook.name}</p>
				</div>
				<button class="icon-picker-close" aria-label="Close icon picker" onclick={() => iconPickerNotebook = null}>
					<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M18 6 6 18M6 6l12 12" /></svg>
				</button>
			</header>
			<div class="icon-picker-grid">
				{#each iconPickerExpanded ? NOTEBOOK_ICON_OPTIONS : NOTEBOOK_ICON_OPTIONS.slice(0, COMMON_NOTEBOOK_ICON_COUNT) as option (option.id)}
					<button
						class="icon-picker-option"
						class:active={iconPickerActive === option.id}
						style:color={iconPickerColor}
						aria-label={option.label}
						title={option.label}
						onclick={() => handleBuiltinIcon(iconPickerNotebook!, option.id)}
					>
						<NotebookGlyph icon={option.id} size={20} />
						<span>{option.label}</span>
					</button>
				{/each}
				<button class="icon-picker-more" aria-expanded={iconPickerExpanded} onclick={() => iconPickerShowAll = !iconPickerExpanded}>
					{iconPickerExpanded ? 'Fewer icons' : 'More icons'}
				</button>
			</div>
			<div class="icon-color-section">
				<span class="icon-color-label">Color</span>
				<div class="icon-swatches">
					<button
						class="icon-swatch none"
						class:active={!iconPickerColor}
						aria-label="Use default color"
						title="Default color"
						onclick={() => handleIconColor(iconPickerNotebook!, null)}
					></button>
					{#each TAG_COLOR_PRESETS as preset (preset)}
						<button
							class="icon-swatch"
							class:active={iconPickerColor === preset}
							style:background={preset}
							aria-label={`Use ${preset}`}
							title={preset}
							onclick={() => handleIconColor(iconPickerNotebook!, preset)}
						></button>
					{/each}
					<label class="icon-swatch custom" title="Custom color">
						<input
							type="color"
							value={iconPickerColor?.length === 7 ? iconPickerColor : '#5b6abf'}
							oninput={(e) => handleIconColor(iconPickerNotebook!, e.currentTarget.value)}
							aria-label="Custom icon color"
						/>
					</label>
				</div>
			</div>
			<div class="icon-picker-actions">
				<button onclick={() => handleCustomIcon(iconPickerNotebook!)}>
					<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2" /><circle cx="8.5" cy="8.5" r="1.5" /><path d="m21 15-5-5L5 21" /></svg>
					Custom image...
				</button>
				{#if $notebookIcons[normalizeNotebookIconKey(iconPickerNotebook.relative_path)]}
					<button class="remove" onclick={() => handleRemoveIcon(iconPickerNotebook!)}>
						<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><path d="M4 7h16M9 7V4h6v3M7 7l1 13h8l1-13" /></svg>
						Use default
					</button>
				{/if}
				<p class="icon-picker-recommendation">Colors do not apply to custom images. Recommended image size: <strong>1:1 aspect ratio</strong></p>
			</div>
		</div>
	</div>
{/if}

{#if tagContextMenu}
	{#if isMobile}
		<button type="button" class="context-menu-backdrop" aria-label="Close tag actions" onclick={() => tagContextMenu = null}></button>
	{/if}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="context-menu" class:mobile={isMobile} style="left: {tagContextMenu.x}px; top: {tagContextMenu.y}px" onmousedown={(e) => e.stopPropagation()}>
		<button onclick={() => openTagStylePicker(tagContextMenu!.tag)}>
			<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10" /><path d="M8 14s1.5 2 4 2 4-2 4-2" /><line x1="9" y1="9" x2="9.01" y2="9" /><line x1="15" y1="9" x2="15.01" y2="9" /></svg>
			Customize...
		</button>
	</div>
{/if}

{#if stylePickerTag}
	<TagStylePicker tag={stylePickerTag} onclose={() => stylePickerTag = null} />
{/if}

{#if trashContextMenu}
	{#if isMobile}
		<button type="button" class="context-menu-backdrop" aria-label="Close trash actions" onclick={() => trashContextMenu = null}></button>
	{/if}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="context-menu" class:mobile={isMobile} style="left: {trashContextMenu.x}px; top: {trashContextMenu.y}px" onmousedown={(e) => e.stopPropagation()}>
		<button class="danger" onclick={handleEmptyTrash}>
			<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
				<polyline points="3 6 5 6 21 6" /><path d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2" /><line x1="10" y1="11" x2="10" y2="17" /><line x1="14" y1="11" x2="14" y2="17" />
			</svg>
			Empty Trash
		</button>
	</div>
{/if}

{#if deleteConfirm}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="delete-confirm-overlay" onclick={dismissDeleteConfirm} onkeydown={(e) => { if (e.key === 'Escape') deleteConfirm = null; }}>
		<div class="delete-confirm" class:mobile={isMobile} role="alertdialog" aria-modal="true" aria-labelledby="delete-confirm-title" tabindex="-1">
			<h4 id="delete-confirm-title">Delete "{deleteConfirm.name}"?</h4>
			<p>This notebook contains {countNotesRecursive(deleteConfirm)} note{countNotesRecursive(deleteConfirm) === 1 ? '' : 's'} that will be permanently deleted.</p>
			<div class="delete-confirm-actions">
				<button class="delete-confirm-cancel" onclick={() => deleteConfirm = null}>Cancel</button>
				<button class="delete-confirm-btn" onclick={() => confirmDelete(deleteConfirm!)}>Delete</button>
			</div>
		</div>
	</div>
{/if}

{#snippet tagRow(tag: string, count: number)}
	<div class="tag-row">
		<button
			class="tag-item"
			class:active={$viewMode === 'tag' && $activeTag === tag}
			onclick={() => selectTag(tag)}
			oncontextmenu={(e) => onTagContextMenu(e, tag)}
		>
			<span class="tag-label-slot">
				<TagLabel name={tag} size={isMobile ? 16 : 14} />
			</span>
			<span class="tag-count">{count}</span>
		</button>
		{#if isMobile}
			<button
				type="button"
				class="notebook-actions-btn"
				aria-label={`Customize ${tag}`}
				onclick={(e) => openTagMenu(e, tag)}
			>
				<svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><circle cx="5" cy="12" r="1.8"/><circle cx="12" cy="12" r="1.8"/><circle cx="19" cy="12" r="1.8"/></svg>
			</button>
		{/if}
	</div>
{/snippet}

{#snippet notebookItem(nb: NotebookEntry, depth: number)}
	{@const hasChildren = nb.children.length > 0}
	{@const isCollapsed = $collapsedNotebooks.includes(nb.path)}
	{@const iconSrc = getNotebookIconSrc(nb)}
	{@const builtinIcon = decodeNotebookIcon($notebookIcons[normalizeNotebookIconKey(nb.relative_path)])}
	{#if editingNotebook === nb.path}
		<div class="notebook-item" style="padding-left: {4 + depth * 16}px">
			<input
				type="text"
				class="rename-input"
				bind:this={renameInput}
				bind:value={editValue}
				onkeydown={(e) => {
					if (e.key === 'Enter') handleRename(nb);
					if (e.key === 'Escape') editingNotebook = null;
				}}
				onblur={() => handleRename(nb)}
			/>
		</div>
	{:else}
		<div class="notebook-row" data-nb-path={nb.path}>
		<button
			class="notebook-item"
			class:active={$viewMode === 'notebook' && $activeNotebook?.path === nb.path}
			class:drop-target={dropTargetPath === nb.path && dropPosition === 'into'}
			class:drop-above={dropTargetPath === nb.path && dropPosition === 'above'}
			class:drop-below={dropTargetPath === nb.path && dropPosition === 'below'}
			style="padding-left: {4 + depth * 16}px"
			data-nb-path={nb.path}
			onclick={(e) => {
				if (!(e.target as Element).closest('.nb-drag-handle')) selectNotebook(nb);
			}}
			onkeydown={(e) => {
				if (e.key === 'F2') {
					e.preventDefault();
					startRename(nb);
				}
			}}
			oncontextmenu={(e) => onContextMenu(e, nb)}
			ondragover={(e) => {
				e.preventDefault();
				if (draggedNotebookPath) {
					// Prevent drop on self or descendant
					if (nb.path === draggedNotebookPath || nb.path.startsWith(draggedNotebookPath + '/')) {
						e.dataTransfer!.dropEffect = 'none';
						return;
					}
				}
				// Decide drop position from cursor Y within the row: top 30% = above, bottom 30% = below, middle = into
				const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
				const relY = (e.clientY - rect.top) / rect.height;
				let pos: 'above' | 'into' | 'below';
				if (draggedNotebookPath && relY < 0.3) pos = 'above';
				else if (draggedNotebookPath && relY > 0.7) pos = 'below';
				else pos = 'into';
				// For 'into', reject if already in that parent (no-op)
				if (pos === 'into' && draggedNotebookPath) {
					const parentDir = draggedNotebookPath.substring(0, draggedNotebookPath.lastIndexOf('/'));
					if (parentDir === nb.path) { e.dataTransfer!.dropEffect = 'none'; return; }
				}
				e.dataTransfer!.dropEffect = 'move';
				dropTargetPath = nb.path;
				dropPosition = pos;
			}}
			ondragleave={() => { if (dropTargetPath === nb.path) { dropTargetPath = null; dropPosition = null; } }}
			ondrop={(e) => {
				e.preventDefault();
				if (draggedNotebookPath) {
					// Recompute the drop position from the drop coordinates, not the dragover-set
					// state: dragleave over the row's child nodes (chevron/icon/label) clears
					// dropPosition before drop (esp. Windows WebView2), which mis-routed every drop
					// to "nest" and broke reorder + un-nest. (issue #97)
					const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
					const relY = (e.clientY - rect.top) / rect.height;
					const pos = relY < 0.3 ? 'above' : relY > 0.7 ? 'below' : 'into';
					if (pos === 'above' || pos === 'below') {
						handleNotebookReorder(draggedNotebookPath, nb.path, pos);
					} else {
						handleNotebookDrop(e, nb.path);
					}
				} else {
					handleNoteDrop(e, nb);
				}
			}}
		>
			{#if hasChildren}
				<!-- svelte-ignore a11y_no_static_element_interactions -->
				<span class="chevron" class:collapsed={isCollapsed} onclick={(e) => toggleCollapse(nb, e)} onkeydown={(e) => { if (e.key === 'Enter') toggleCollapse(nb, e); }}>
					<svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						<polyline points="6 9 12 15 18 9" />
					</svg>
				</span>
			{:else}
				<span class="chevron-spacer"></span>
			{/if}
		<span class="notebook-icon-slot" class:notebook-builtin-icon={!!builtinIcon?.icon} style:color={builtinIcon?.color}>
		{#if builtinIcon?.icon}
			<NotebookGlyph icon={builtinIcon.icon} size={18} />
		{:else if iconSrc}
			<img class="notebook-icon" crossorigin="anonymous" src={iconSrc} alt="" />
		{:else if isCollapsed || nb.children.length === 0}
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="flex-shrink:0" style:opacity={builtinIcon?.color ? 1 : 0.6}>
				<path d="M2 6h4"/><path d="M2 10h4"/><path d="M2 14h4"/><path d="M2 18h4"/>
				<path d="M6 2h12a2 2 0 012 2v16a2 2 0 01-2 2H6a2 2 0 01-2-2V4a2 2 0 012-2z"/>
			</svg>
		{:else}
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="flex-shrink:0" style:opacity={builtinIcon?.color ? 1 : 0.6}>
				<path d="M2 6h4"/><path d="M2 10h4"/><path d="M2 14h4"/><path d="M2 18h4"/>
				<path d="M6 2h12a2 2 0 012 2v16a2 2 0 01-2 2H6a2 2 0 01-2-2V4a2 2 0 012-2z"/>
				<path d="M10 9h8"/><path d="M10 13h8"/><path d="M10 17h5"/>
			</svg>
		{/if}
		</span>
			<span class="notebook-name">{nb.name} <span class="notebook-count">{nb.note_count}</span></span>
			{#if $notebookSortMode === 'manual'}
				<!-- svelte-ignore a11y_no_static_element_interactions -->
				<span
					class="nb-drag-handle"
					title="Drag to reorder"
					onpointerdown={(e) => nbHandleDown(e, nb)}
				>
					<svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor"><circle cx="9" cy="5" r="1.6"/><circle cx="15" cy="5" r="1.6"/><circle cx="9" cy="12" r="1.6"/><circle cx="15" cy="12" r="1.6"/><circle cx="9" cy="19" r="1.6"/><circle cx="15" cy="19" r="1.6"/></svg>
				</span>
			{/if}
		</button>
		{#if isMobile}
			<button
				type="button"
				class="notebook-actions-btn"
				aria-label={`Actions for ${nb.name}`}
				onclick={(e) => openNotebookMenu(e, nb)}
			>
				<svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><circle cx="5" cy="12" r="1.8"/><circle cx="12" cy="12" r="1.8"/><circle cx="19" cy="12" r="1.8"/></svg>
			</button>
		{/if}
		</div>
	{/if}
	{#if hasChildren && !isCollapsed}
		{#each nb.children as child (child.path)}
			{@render notebookItem(child, depth + 1)}
		{/each}
	{/if}
{/snippet}

<style>
	.sidebar-error {
		padding: 8px;
		font-size: 12px;
		color: var(--text-secondary);
		overflow-wrap: anywhere;
	}

	.sidebar {
		display: flex;
		flex-direction: column;
		height: 100%;
		background: var(--bg-secondary);
		border-right: 1px solid var(--border-color);
		overflow: hidden;
		min-width: 44px;
		user-select: none;
	}

	.sidebar.collapsed {
		width: 44px !important;
		min-width: 44px;
		max-width: 44px;
	}

	/* Collapsed rail: stack the toolbar icons vertically - search under the expand chevron at
	   the top, info + settings pinned to the bottom. */
	.sidebar.collapsed .sidebar-header {
		flex-direction: column;
		justify-content: flex-start;
		gap: 4px;
		padding: 8px 4px;
		border-bottom: none;
	}

	.sidebar.collapsed .sidebar-footer {
		flex-direction: column;
		align-items: center;
		gap: 4px;
		margin-top: auto;
		padding: 8px 4px;
		border-top: none;
	}

	.sidebar.collapsed .sidebar-footer-right {
		flex-direction: column;
	}

	.sidebar-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 10px 8px;
		border-bottom: 1px solid var(--border-light);
		flex-shrink: 0;
	}

	.sidebar-nav {
		padding: 8px 6px;
		flex-shrink: 0;
	}

	.nav-item {
		display: flex;
		align-items: center;
		gap: 8px;
		width: 100%;
		padding: 6px 10px;
		border: none;
		background: none;
		border-radius: 6px;
		color: var(--text-secondary);
		font-size: inherit;
		cursor: pointer;
		transition: all 0.1s;
	}

	.nav-item:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.nav-item.active {
		background: var(--accent-light);
		color: var(--text-accent);
	}

	.section {
		flex: 1;
		overflow-y: auto;
		padding: 0 0 4px;
		border-top: 1px solid var(--border-light);
	}

	/* With the top nav hidden, the header's border-bottom is the only divider needed -
	   drop the section's border-top so there's no doubled line under the header. */
	.sidebar.nav-empty .section {
		border-top: none;
	}

	/* Pin the Notebooks header so its collapse/expand-all button stays reachable while the tree
	   scrolls; the section's border-top draws the divider above it, matching the Tags section. (issue #126) */
	.section > .section-header {
		position: sticky;
		top: 0;
		z-index: 1;
		background: var(--bg-secondary);
	}

	.tags-section {
		flex-shrink: 0;
		border-top: 1px solid var(--border-light);
		padding: 4px 0;
		max-height: 30%;
		overflow-y: auto;
	}

	.section-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 8px 12px 4px;
		width: 100%;
		background: none;
		border: none;
		cursor: pointer;
		color: inherit;
		font: inherit;
		text-align: left;
	}

	.section-title {
		font-size: 11px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--text-tertiary);
	}

	.section-actions {
		display: flex;
		align-items: center;
		gap: 2px;
	}
	.icon-btn, .icon-btn-sm {
		background: none;
		border: none;
		color: var(--text-tertiary);
		cursor: pointer;
		padding: 4px;
		border-radius: 4px;
		display: flex;
		align-items: center;
		justify-content: center;
	}

	.icon-btn:hover, .icon-btn-sm:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.collapse-btn {
		background: none;
		border: none;
		color: var(--text-tertiary);
		cursor: pointer;
		padding: 4px;
		border-radius: 4px;
		display: flex;
		align-items: center;
	}

	.collapse-btn:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.new-notebook-input {
		padding: 2px 8px;
		display: flex;
		align-items: center;
		gap: 0;
		border: 1px solid var(--accent);
		border-radius: 4px;
		background: var(--bg-primary);
		overflow: hidden;
	}

	.new-notebook-parent {
		display: inline-flex;
		align-items: center;
		gap: 2px;
		margin: 0 2px 0 6px;
		padding: 2px 5px;
		color: var(--text-tertiary);
		font-size: inherit;
		white-space: nowrap;
		flex-shrink: 0;
		border: none;
		border-radius: 4px;
		background: var(--bg-hover);
		cursor: pointer;
	}

	.new-notebook-parent:hover {
		color: var(--text-primary);
		background: var(--accent-light);
	}

	.new-notebook-parent-x {
		font-size: 13px;
		line-height: 1;
		opacity: 0.7;
	}

	.new-notebook-input input {
		flex: 1;
		min-width: 0;
		padding: 4px 8px;
		border: none;
		background: transparent;
		color: var(--text-primary);
		font-size: inherit;
		outline: none;
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

	.notebook-list {
		padding: 0 2px;
	}

	.notebook-row {
		position: relative;
	}

	.notebook-item {
		display: flex;
		align-items: center;
		gap: 6px;
		width: 100%;
		padding: 5px 8px 5px 4px;
		border: none;
		background: none;
		border-radius: 6px;
		color: var(--text-secondary);
		font-size: inherit;
		cursor: pointer;
		transition: all 0.1s;
	}

	.notebook-item:hover {
		background: var(--bg-hover);
	}

	.notebook-item.active {
		background: var(--accent-light);
		color: var(--text-accent);
	}

	.notebook-item.drop-target,
	.section-header.drop-target {
		background: color-mix(in srgb, var(--accent) 20%, transparent);
		outline: 2px dashed var(--accent);
		outline-offset: -2px;
		border-radius: 6px;
	}
	.notebook-item.drop-above {
		box-shadow: 0 -2px 0 0 var(--accent);
	}
	.notebook-item.drop-below {
		box-shadow: 0 2px 0 0 var(--accent);
	}

	.chevron {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 14px;
		height: 14px;
		flex-shrink: 0;
		color: var(--text-tertiary);
		transition: transform 0.15s ease;
		cursor: pointer;
		border-radius: 3px;
	}

	.chevron:hover {
		color: var(--text-primary);
	}

	.chevron.collapsed {
		transform: rotate(-90deg);
	}

	.chevron-spacer {
		width: 14px;
		flex-shrink: 0;
	}

	.notebook-icon {
		width: 20px;
		height: 20px;
		border-radius: 3px;
		object-fit: cover;
		flex-shrink: 0;
	}

	.notebook-icon-slot {
		width: 20px;
		height: 20px;
		display: flex;
		align-items: center;
		justify-content: center;
		flex-shrink: 0;
	}

	.notebook-builtin-icon {
		color: var(--text-secondary);
	}

	.notebook-name {
		flex: 1;
		text-align: left;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.notebook-count {
		font-size: 11px;
		color: var(--text-tertiary);
		margin-left: 2px;
	}

	.nb-drag-handle {
		display: flex;
		align-items: center;
		justify-content: center;
		flex: 0 0 auto;
		padding: 2px;
		margin-left: 2px;
		color: var(--text-tertiary);
		opacity: 0.45;
		cursor: grab;
		touch-action: none;
	}

	.nb-drag-handle:hover {
		opacity: 1;
	}

	.nb-drag-handle:active {
		cursor: grabbing;
	}

	.notebook-actions-btn {
		display: flex;
		align-items: center;
		justify-content: center;
		position: absolute;
		right: 4px;
		top: 50%;
		transform: translateY(-50%);
		width: 44px;
		height: 44px;
		padding: 0;
		border: none;
		border-radius: 8px;
		background: none;
		color: var(--text-tertiary);
	}

	.notebook-actions-btn:active {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.tag-list {
		padding: 0 4px;
	}

	.tag-row {
		position: relative;
	}

	.tag-item {
		display: flex;
		align-items: center;
		gap: 4px;
		width: 100%;
		padding: 4px 12px;
		border: none;
		background: none;
		border-radius: 6px;
		color: var(--text-secondary);
		font-size: inherit;
		cursor: pointer;
		transition: all 0.1s;
	}

	.tag-label-slot {
		flex: 1;
		min-width: 0;
		display: flex;
	}

	.tag-item:hover {
		background: var(--bg-hover);
	}

	.tag-item.active {
		background: var(--accent-light);
		color: var(--text-accent);
	}

	.tag-count {
		font-size: 11px;
		color: var(--text-tertiary);
	}

	.sidebar-footer {
		padding: 8px;
		border-top: 1px solid var(--border-light);
		display: flex;
		justify-content: space-between;
		align-items: center;
		flex-shrink: 0;
	}

	.sidebar-footer-right {
		display: flex;
		gap: 4px;
	}

	.context-menu {
		position: fixed;
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 8px;
		box-shadow: var(--shadow-md);
		padding: 4px;
		z-index: 1000;
		min-width: 140px;
	}

	.context-menu-backdrop {
		position: fixed;
		inset: 0;
		z-index: 999;
		padding: 0;
		border: none;
		background: rgba(0, 0, 0, 0.18);
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

	.icon-picker-overlay {
		position: fixed;
		inset: 0;
		z-index: 1100;
		display: grid;
		place-items: center;
		padding: 20px;
	}

	.icon-picker-backdrop {
		position: absolute;
		inset: 0;
		width: 100%;
		height: 100%;
		padding: 0;
		border: none;
		background: rgba(8, 10, 18, 0.42);
		cursor: default;
	}

	.icon-picker {
		position: relative;
		z-index: 1;
		width: min(360px, 100%);
		max-height: calc(100dvh - 40px);
		overflow-y: auto;
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 14px;
		box-shadow: var(--shadow-lg);
	}

	.icon-picker-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 16px;
		padding: 16px 18px 12px;
		border-bottom: 1px solid var(--border-light);
	}

	.icon-picker-header h3,
	.icon-picker-header p {
		margin: 0;
	}

	.icon-picker-header h3 {
		color: var(--text-primary);
		font-size: 15px;
		font-weight: 650;
	}

	.icon-picker-header p {
		margin-top: 2px;
		color: var(--text-tertiary);
		font-size: 12px;
	}

	.icon-picker-close {
		width: 32px;
		height: 32px;
		display: grid;
		place-items: center;
		padding: 0;
		border: none;
		border-radius: 8px;
		background: transparent;
		color: var(--text-tertiary);
		cursor: pointer;
	}

	.icon-picker-close:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.icon-picker-grid {
		display: grid;
		grid-template-columns: repeat(4, minmax(0, 1fr));
		gap: 7px;
		padding: 14px;
	}

	.icon-picker-option {
		min-width: 0;
		min-height: 60px;
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		gap: 5px;
		padding: 7px 3px 6px;
		border: 1px solid transparent;
		border-radius: 9px;
		background: var(--bg-secondary);
		color: var(--text-secondary);
		cursor: pointer;
	}

	.icon-picker-option span {
		max-width: 100%;
		overflow: hidden;
		color: var(--text-tertiary);
		font-size: 10px;
		line-height: 1.1;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.icon-picker-option:hover {
		border-color: var(--border-color);
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.icon-picker-option.active {
		border-color: var(--accent);
		background: var(--accent-light);
		color: var(--accent);
	}

	.icon-picker-option.active span {
		color: var(--accent);
	}

	.icon-picker-more {
		grid-column: 1 / -1;
		padding: 7px;
		border: none;
		border-radius: 8px;
		background: transparent;
		color: var(--text-tertiary);
		font-size: 12px;
		cursor: pointer;
	}

	.icon-picker-more:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.icon-color-section {
		display: flex;
		flex-direction: column;
		gap: 8px;
		padding: 0 14px 12px;
	}

	.icon-color-label {
		color: var(--text-secondary);
		font-size: 12px;
		font-weight: 600;
	}

	.icon-swatches {
		display: flex;
		flex-wrap: wrap;
		gap: 7px;
		align-items: center;
	}

	.icon-swatch {
		width: 22px;
		height: 22px;
		padding: 0;
		border: 2px solid transparent;
		border-radius: 50%;
		background: var(--bg-tertiary);
		cursor: pointer;
	}

	.icon-swatch:hover,
	.icon-swatch.active {
		border-color: var(--text-primary);
	}

	.icon-swatch.none {
		background:
			linear-gradient(to bottom right, transparent calc(50% - 1px), var(--text-tertiary) calc(50% - 1px), var(--text-tertiary) calc(50% + 1px), transparent calc(50% + 1px)),
			var(--bg-secondary);
		border-color: var(--border-color);
	}

	.icon-swatch.custom {
		display: grid;
		place-items: center;
		overflow: hidden;
		background: conic-gradient(from 90deg, #e11d48, #eab308, #22c55e, #3b82f6, #8b5cf6, #e11d48);
	}

	.icon-swatch.custom input {
		width: 28px;
		height: 28px;
		padding: 0;
		border: none;
		background: none;
		cursor: pointer;
		transform: scale(1.4);
	}

	.icon-picker-actions {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
		padding: 12px 14px 14px;
		border-top: 1px solid var(--border-light);
	}

	.icon-picker-recommendation {
		flex: 0 0 100%;
		margin: 2px 0 0;
		color: var(--text-tertiary);
		font-size: 11px;
		line-height: 1.3;
		text-align: center;
	}

	.icon-picker-actions button {
		flex: 1;
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 7px;
		padding: 9px 12px;
		border: 1px solid var(--border-color);
		border-radius: 8px;
		background: var(--bg-secondary);
		color: var(--text-secondary);
		font-size: 12px;
		cursor: pointer;
	}

	.icon-picker-actions button:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.icon-picker-actions button.remove {
		background: transparent;
	}

	.icon-picker-overlay.mobile {
		place-items: end center;
		padding:
			20px
			calc(12px + env(safe-area-inset-right, 0px))
			calc(12px + env(safe-area-inset-bottom, 0px))
			calc(12px + env(safe-area-inset-left, 0px));
	}

	.icon-picker-overlay.mobile .icon-picker {
		width: 100%;
		max-height: calc(100dvh - 32px - env(safe-area-inset-top, 0px) - env(safe-area-inset-bottom, 0px));
	}

	.icon-picker-overlay.mobile .icon-picker-option {
		min-height: 64px;
	}

	.icon-picker-overlay.mobile .icon-picker-more {
		min-height: 44px;
		font-size: 14px;
	}

	.icon-picker-overlay.mobile .icon-picker-actions button {
		min-height: 44px;
		font-size: 14px;
	}

	.delete-confirm-overlay {
		position: fixed;
		inset: 0;
		background: rgba(0, 0, 0, 0.4);
		display: flex;
		align-items: center;
		justify-content: center;
		z-index: 1000;
	}

	.delete-confirm {
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 10px;
		padding: 20px 24px;
		max-width: 340px;
		box-shadow: 0 8px 32px rgba(0, 0, 0, 0.25);
	}

	.delete-confirm h4 {
		margin: 0 0 8px;
		font-size: 14px;
		color: var(--text-primary);
	}

	.delete-confirm p {
		margin: 0 0 16px;
		font-size: 12.5px;
		color: var(--text-secondary);
		line-height: 1.4;
	}

	.delete-confirm-actions {
		display: flex;
		gap: 8px;
		justify-content: flex-end;
	}

	.delete-confirm-cancel {
		padding: 6px 14px;
		border: 1px solid var(--border-color);
		border-radius: 6px;
		background: var(--bg-secondary);
		color: var(--text-primary);
		font-size: 12px;
		cursor: pointer;
	}

	.delete-confirm-cancel:hover {
		background: var(--bg-hover);
	}

	.delete-confirm-btn {
		padding: 6px 14px;
		border: none;
		border-radius: 6px;
		background: var(--danger, #e53e3e);
		color: white;
		font-size: 12px;
		font-weight: 600;
		cursor: pointer;
	}

	.delete-confirm-btn:hover {
		opacity: 0.9;
	}

	/* ═══ MOBILE (class-based for Android high-DPI) ═══ */
	.sidebar.mobile {
		width: 100% !important;
		min-width: 100%;
		border-right: none;
	}

	.sidebar.mobile.collapsed {
		width: 100% !important;
		min-width: 100%;
		max-width: 100%;
	}

	.sidebar.mobile .sidebar-header {
		padding: 10px 12px;
	}

	.sidebar.mobile .collapse-btn {
		display: none;
	}

	.sidebar.mobile .nav-item {
		padding: 12px 14px;
		font-size: 15px;
		min-height: 48px;
		gap: 12px;
	}

	.sidebar.mobile .nav-item svg {
		width: 20px;
		height: 20px;
	}

	.sidebar.mobile .section {
		padding-bottom: 180px;
	}

	.sidebar.mobile .tags-section-inline {
		border-top: 1px solid var(--border-light);
		padding: 4px 0;
		margin-top: 8px;
	}

	.sidebar.mobile .section-header {
		padding: 12px 16px 6px;
		min-height: 44px;
	}

	.sidebar.mobile .section-title {
		font-size: 12px;
	}

	.sidebar.mobile .icon-btn,
	.sidebar.mobile .icon-btn-sm {
		min-width: 44px;
		min-height: 44px;
		padding: 10px;
	}

	.sidebar.mobile .notebook-item {
		padding: 2px 56px 2px 12px;
		min-height: 48px;
		gap: 10px;
		font-size: 15px;
	}

	.sidebar.mobile .notebook-icon {
		width: 24px;
		height: 24px;
	}

	.sidebar.mobile .notebook-icon-slot {
		width: 24px;
		height: 24px;
	}

	.sidebar.mobile .new-notebook-input {
		padding: 4px 12px;
	}

	.sidebar.mobile .new-notebook-parent {
		padding: 10px 0 10px 12px;
		font-size: 15px;
	}

	.sidebar.mobile .new-notebook-input input {
		padding: 10px 12px;
		font-size: 15px;
	}

	.sidebar.mobile .rename-input {
		padding: 10px 12px;
		font-size: 15px;
	}

	.sidebar.mobile .tag-item {
		padding: 10px 52px 10px 16px;
		min-height: 44px;
		font-size: 14px;
	}

	.sidebar.mobile .sidebar-footer {
		padding: 8px 12px;
		display: none;
	}

	.context-menu.mobile {
		left: calc(12px + env(safe-area-inset-left, 0px)) !important;
		right: calc(12px + env(safe-area-inset-right, 0px));
		top: auto !important;
		bottom: calc(12px + env(safe-area-inset-bottom, 0px));
		min-width: 0;
		max-height: calc(100dvh - 24px - env(safe-area-inset-top, 0px) - env(safe-area-inset-bottom, 0px));
		overflow-y: auto;
		border-radius: 12px;
		padding: 6px;
	}

	.context-menu.mobile button {
		padding: 12px 16px;
		font-size: 15px;
		min-height: 44px;
		border-radius: 8px;
	}

	.delete-confirm.mobile {
		max-width: calc(100vw - 40px);
		padding: 24px;
	}

	.delete-confirm.mobile h4 {
		font-size: 16px;
	}

	.delete-confirm.mobile p {
		font-size: 14px;
	}

	.delete-confirm.mobile .delete-confirm-cancel,
	.delete-confirm.mobile .delete-confirm-btn {
		padding: 10px 20px;
		font-size: 14px;
		min-height: 44px;
	}
</style>
