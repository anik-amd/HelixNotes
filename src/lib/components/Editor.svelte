<script lang="ts">
	import { onDestroy, tick, untrack } from 'svelte';
	import { get } from 'svelte/store';
	import { Editor } from '@tiptap/core';
	import StarterKit from '@tiptap/starter-kit';
	import TaskList from '@tiptap/extension-task-list';
	import TaskItem from '@tiptap/extension-task-item';
	import { Table } from '@tiptap/extension-table';
	import { TableRow } from '@tiptap/extension-table-row';
	import { TableCell } from '@tiptap/extension-table-cell';
	import { TableHeader } from '@tiptap/extension-table-header';
	import Link from '@tiptap/extension-link';
	import Image from '@tiptap/extension-image';
	import Highlight from '@tiptap/extension-highlight';
	import Typography from '@tiptap/extension-typography';
	import Underline from '@tiptap/extension-underline';
	import Subscript from '@tiptap/extension-subscript';
	import Superscript from '@tiptap/extension-superscript';
	import { Color } from '@tiptap/extension-color';
	import { TextStyle } from '@tiptap/extension-text-style';
	import { CachedCodeBlockLowlight } from '$lib/editor/extensions/cachedCodeBlockLowlight';
	import { Details, DetailsSummary, DetailsContent } from '@tiptap/extension-details';
	import TextAlign from '@tiptap/extension-text-align';
	import { common, createLowlight } from 'lowlight';
	import powershell from 'highlight.js/lib/languages/powershell';
	import katex from 'katex';
	import 'katex/dist/katex.min.css';
	import { Extension, Node as TiptapNode, Mark as TiptapMark, mergeAttributes } from '@tiptap/core';
	import { Plugin, PluginKey, EditorState, Selection, TextSelection } from '@tiptap/pm/state';
	import { Decoration, DecorationSet } from '@tiptap/pm/view';
	import { DOMParser as PMDOMParser, DOMSerializer, type Node as ProseMirrorNode } from '@tiptap/pm/model';
	import { convertFileSrc } from '@tauri-apps/api/core';
	import { getCurrentWindow } from '@tauri-apps/api/window';
	import { readFile } from '@tauri-apps/plugin-fs';
	import { openFile, openUrl, copyFileTo, copyImageToClipboard as copyImageToClipboardCmd, writeBytesTo, copyPngToClipboard, copyTextToClipboard } from '$lib/api';
	import { save as saveDialog } from '@tauri-apps/plugin-dialog';
	import { activeNote, activeNotePath, appConfig, editorDirty, sourceMode, focusMode, readOnly, quickAccessPaths, notes, navHistory, canGoBack, canGoForward, viewerNote, viewMode, notebooks, outlineWidth } from '$lib/stores/app';
	import { saveNote, importNote, createNote, saveImage, saveAttachment, readClipboardImage, readClipboardContents, addQuickAccess, removeQuickAccess, getQuickAccess, getNoteVersions, getNoteVersionContent, createVersion, aiAsk, getAllNoteTitles, readNote, renameNote, setToolbarLayout } from '$lib/api';
	import type { NoteContent, VersionEntry, AiStreamEvent, NoteTitleEntry, TaskItem as TaskRecord, FileEvent, SearchTarget, ToolbarLayout } from '$lib/types';
	import { listen } from '@tauri-apps/api/event';
	import { debounce } from '$lib/utils/debounce';
	import { formatClockTime } from '$lib/utils/time';
	import DOMPurify from 'dompurify';
	import { splitSnapshot } from '$lib/utils/history-comparison';
	import HistoryReview from './HistoryReview.svelte';
	import { RevisionSaveQueue, type SaveRevision } from '$lib/utils/revision-save';
	import { isCurrentVaultEvent } from '$lib/utils/note-list-updates';
	import { beginNoteNavigation, isCurrentNoteNavigation, cancelNoteNavigation } from '$lib/utils/note-navigation';
	import { encryptSecretText, decryptSecretText, readSecretTitle } from '$lib/utils/secrets';
	import { WrapSelectedText } from '$lib/editor/extensions/wrapSelectedText';
	import { CodeBlockInputScroll } from '$lib/editor/extensions/codeBlockInputScroll';
	import { ImageGrid } from '$lib/editor/extensions/imageGrid';
	import { defaultGridCols, expandImageRange, reorderTarget, serializeImgGrid } from '$lib/editor/imageGrid';
	import { calloutGroup, calloutIcon, calloutLabel, CALLOUT_MENU, transformCalloutBlockquotes, serializeCallout } from '$lib/editor/callouts';
	import SourceEditor from './SourceEditor.svelte';
	import { SourceController } from '$lib/editor/source/controller';
	import { findDocumentMatches, findSourceMatches, closestSearchMatch, type NoteSearchMatch, replaceableMatches } from '$lib/editor/search';
	import { keybindings, bindingToKeys } from '$lib/keybindings';
	import { resolveSearchTarget, visibleSearchTarget } from '$lib/editor/search-target';
	import { convertListNode, type MixedListName } from '$lib/editor/mixedLists';
	import { clearFormatting } from '$lib/editor/clearFormatting';
	import { toolbarLabels, resolveToolbarLayout, groupToolbarButtons, moveToolbarButton, toolbarLayoutPreference, type ToolbarButtonId, type ToolbarArea, type ResolvedToolbarLayout } from '$lib/editor/toolbar';
	import { buildCodeCopyDecorations, buildTaskMetaDecorations, focusedPlaceholderDecorations } from '$lib/editor/decorations';
	import { escapeBlockStarts, serializeImageTitle, serializeInlineMarkdownKeepingDollars, serializeCodeBlock, joinMarkdownBlocks, joinListItemBlocks, type ListItemBlock, type MarkdownBlock } from '$lib/editor/markdown';
	import { restoreTitleHeading, stripTitleHeading, type HiddenTitleHeading } from '$lib/editor/titleVisibility';
	import { documentHeadings, sourceHeadings, activeHeadingIndex, mapDocumentHeadings, mapSourceHeadings, type OutlineHeading } from '$lib/editor/outline';
	import { createEditorMarkdownParser } from '$lib/editor/markdownParser';
	import { collectClipboardPayload, isEmptyPayload, needsNativeClipboard, withNativeClipboard, type ClipboardPastePayload } from '$lib/utils/clipboard-paste';
	import { externalNoteReferences } from '$lib/editor/externalNote';
	import { tagIterationKey } from '$lib/utils/tag-styles';
	import { replaceWithWikiLink } from '$lib/editor/wikiLinks';
	import { renamedLink, renamedLinkChanges, type LinkRename } from '$lib/editor/renamedLinks';
	import { assetSourceToMarkdown, assetUrlToLocalPath, encodeMarkdownPath, resolveFileReference, noteAssetRoot, isTrashedNote } from '$lib/utils/paths';
	import GraphView from './GraphView.svelte';
	import TagSuggestInput from './TagSuggestInput.svelte';
	import TagLabel from './TagLabel.svelte';
	import ImageViewer from './ImageViewer.svelte';
	import { isMobile, isAndroid } from '$lib/platform';
	import ResizeHandle from './ResizeHandle.svelte';

	const componentId = $props.id();
	const outlinePanelId = `${componentId}-outline`;
	let { onMoveToTrash, onOutlineChange, onShowTag }: {
		onMoveToTrash?: (path: string) => Promise<boolean>;
		onOutlineChange?: (open: boolean, panelId: string) => void;
		/** Shows the notes carrying a tag. Without it (a detached note window) the tags open the tag editor. */
		onShowTag?: (tag: string) => void;
	} = $props();

	const modKey = navigator.platform.startsWith('Mac') ? '⌘' : 'Ctrl';

	// Track virtual keyboard height on mobile via visualViewport
	let keyboardHeight = $state(0);
	$effect(() => {
		if (!isMobile || !window.visualViewport) return;
		const vv = window.visualViewport;
		const update = () => { keyboardHeight = Math.max(0, Math.round(window.innerHeight - vv.height - vv.offsetTop)); };
		update();
		vv.addEventListener('resize', update);
		vv.addEventListener('scroll', update);
		return () => {
			vv.removeEventListener('resize', update);
			vv.removeEventListener('scroll', update);
		};
	});

	let editorElement = $state<HTMLDivElement>(null!);
	const LARGE_DOC_CHARS = 100_000;
	let isLargeDoc = $state(false);
	let editor: Editor | null = null;
	const MixedListShortcuts = Extension.create({
		name: 'mixedListShortcuts',
		priority: 1000,
		addKeyboardShortcuts() {
			return {
				'Mod-Shift-8': () => toggleBulletList(),
				'Mod-Shift-9': () => toggleTaskList(),
			};
		},
	});
	// While a very large note is still being built in slices, its document is partial. Nothing may edit
	// it then; a user action first finishes the build (see loadDocumentProgressively).
	const ProgressiveLoadGuard = Extension.create({
		name: 'progressiveLoadGuard',
		addProseMirrorPlugins() {
			return [new Plugin({ key: new PluginKey('progressiveLoadGuard'), filterTransaction: (tr) => !(progressiveLoad && tr.docChanged) })];
		},
	});
	function hasInlineCode(node: ProseMirrorNode): boolean {
		for (let i = 0; i < node.childCount; i++) {
			if (node.child(i).marks.some(mark => mark.type.name === 'code')) return true;
		}
		return false;
	}
	// The parser marks blocks holding code spans so whitespace-only code survives DOM import.
	const keepCodeWhitespace = {
		parseHTML(this: any) {
			return (this.parent?.() ?? []).flatMap((rule: any) => [
				{ ...rule, tag: `${rule.tag}[data-helix-code]`, preserveWhitespace: 'full' as const },
				rule,
			]);
		},
	};
	const EditorStarterKit = StarterKit.extend({
		addExtensions() {
			// Inline code may carry other marks, so **`code`** and [`code`](url) keep their formatting.
			return this.parent!().map(extension => extension instanceof TiptapMark && extension.name === 'code' ? extension.extend({ excludes: '' })
				: extension instanceof TiptapNode && extension.name === 'listItem' ? extension.extend(keepCodeWhitespace)
				: extension instanceof TiptapNode && (extension.name === 'paragraph' || extension.name === 'heading') ? extension.extend({
				...keepCodeWhitespace,
				renderHTML(props) {
					return this.parent!({
						...props,
						HTMLAttributes: hasInlineCode(props.node)
							? { ...props.HTMLAttributes, 'data-helix-code': '' } : props.HTMLAttributes,
					});
				},
			}) : extension);
		},
	});
	let editorReady = $state(false);
	const trashPreview = $derived(isTrashedNote($activeNotePath, $appConfig?.active_vault));
	// CM retains an immutable document; edits never bind a full source string to Svelte.
	const source = new SourceController((changes, edited) => {
		noteSearchAnchor = changes.mapPos(Math.min(noteSearchAnchor, changes.length));
		if (edited) markEdited();
		scheduleCounts();
		if (noteSearchOpen) scheduleNoteSearch();
		if ($sourceMode && showOutline && outlineOrigin && isOriginCurrent(outlineOrigin)) {
			outlineHeadings = mapSourceHeadings(outlineHeadings, changes);
			scheduleOutline();
			scheduleOutlineActive();
		}
	}, { paste: handleFilePaste, drop: handleFileDrop, scroll: scheduleOutlineActive });
	let loadedPath = '';
	type NoteScrollPosition = { rich: number; source: number };
	const MAX_NOTE_SCROLL_POSITIONS = 200;
	const noteScrollPositions = new Map<string, NoteScrollPosition>();
	let pendingContent = $state<string | null>(null);
	let ignoreNextUpdate = false;
	let isLoadingNote = false;
	let trashingNote = $state(false);
	let renamingNote: SaveRevision | null = null;
	let externalOperationBusy = $state(false);
	let pathMigrationPending = false;
	let lastSourceMode = $sourceMode;
	let linkContextMenu = $state<{ x: number; y: number; href: string; anchor: HTMLAnchorElement } | null>(null);
	let hiddenTitleHeading: HiddenTitleHeading | null = null;
	let targetRevealTimer: ReturnType<typeof setTimeout> | null = null;
	let targetRevealRequest = 0;
	const targetRevealPluginKey = new PluginKey('targetReveal');
	let searchPassageActive = false;
	let searchTitleReveal = $state(false);
	let searchTagsReveal = $state(false);
	let searchRevealNotice = $state('');
	/** Path of a note the rich editor could not display; see failNoteLoad. */
	let loadFailure = $state<string | null>(null);
	type TaskRevealTarget = { sourceOrdinal: number; richOrdinal: number | null };
	const TASK_LINE_RE = /^\s*[-*]\s\[[ xX]\]\s+.+?\s*$/;
	const RICH_TASK_LINE_RE = /^\s*-\s\[(?:x| )\]\s+.+?\s*$/;

	let headingDropdown = $state(false);
	let colorDropdown = $state(false);
	let highlightDropdown = $state(false);
	let alignDropdown = $state(false);
	let insertDropdown = $state(false);
	let tablePickerOpen = $state(false);
	let tablePickerHover = $state({ rows: 0, cols: 0 });
	let formattingBar = $state<HTMLDivElement>();
	let moreButton = $state<HTMLButtonElement>();
	let morePanel = $state<HTMLDivElement>();
	let moreOpen = $state(false);
	let formattingTrigger: HTMLButtonElement | null = null;
	const morePanelId = `${componentId}-more`;
	let toolbarDraft = $state<ResolvedToolbarLayout | null>(null);
	let toolbarSelected = $state<ToolbarButtonId | null>(null);
	let toolbarSaving = $state(false);
	let toolbarSaveError = $state<string | null>(null);
	let toolbarConflict = $state(false);
	let toolbarBase = '';
	let toolbarEditSession = 0;
	let toolbarPendingPreference: string | null = null;
	let toolbarDrag: { id: ToolbarButtonId; pointerId: number; startX: number; startY: number; x: number; y: number; button: HTMLButtonElement; active: boolean } | null = null;
	let toolbarDraggedId = $state<ToolbarButtonId | null>(null);
	let toolbarDrop = $state<{ area: ToolbarArea; index: number; left: number; top: number; height: number } | null>(null);
	let toolbarDragGhost: HTMLElement | null = null;
	let toolbarDragFrame = 0;
	let toolbarCapture: HTMLElement | null = null;
	const toolbarLayout = $derived(toolbarDraft ?? resolveToolbarLayout($appConfig?.toolbar_layout, isMobile, !!$appConfig?.ai_provider));
	const formattingGroups = $derived(groupToolbarButtons(toolbarLayout.main, isMobile));
	const toolbarSelectedArea = $derived(toolbarSelected && toolbarDraft
		? (toolbarDraft.main.includes(toolbarSelected) ? 'main' : toolbarDraft.more.includes(toolbarSelected) ? 'more' : null) : null);
	const toolbarSelectedIndex = $derived(toolbarSelected && toolbarDraft && toolbarSelectedArea
		? toolbarDraft[toolbarSelectedArea].indexOf(toolbarSelected) : -1);
	const toolbarHintId = `${morePanelId}-customize-hint`;

	function scrollEditorBodyToBottom(source: HTMLElement | null | undefined = editorElement) {
		const editorBody = source?.closest('.editor-body') as HTMLElement | null;
		if (!editorBody) return;
		editorBody.scrollTop = editorBody.scrollHeight;
		requestAnimationFrame(() => {
			editorBody.scrollTop = editorBody.scrollHeight;
		});
	}


	function clearTargetReveal() {
		if (targetRevealTimer) clearTimeout(targetRevealTimer);
		targetRevealTimer = null;
		searchPassageActive = false;
		searchTitleReveal = false;
		searchTagsReveal = false;
		searchRevealNotice = '';
		source.highlight(null);
		if (editor && !editor.isDestroyed) {
			editor.view.dispatch(editor.state.tr.setMeta(targetRevealPluginKey, DecorationSet.empty));
		}
	}

	export async function revealSearchTarget(target: SearchTarget | null, canReveal: () => boolean) {
		if (!target || !canReveal()) return;
		closeNoteSearch(false);
		clearTargetReveal();
		const request = ++targetRevealRequest;
		const snapshot = saves.capture();
		const path = loadedPath;
		const vault = $appConfig?.active_vault;
		let mode = $sourceMode;
		const current = () => request === targetRevealRequest && canReveal() && !editorDisposed
			&& path === loadedPath && path === $activeNotePath && vault === $appConfig?.active_vault
			&& mode === $sourceMode && saves.isCurrent(snapshot);
		searchPassageActive = true;
		try {
			await tick();
			if (!current() || !$activeNote) return;
			const changed = () => { searchRevealNotice = 'The indexed passage has changed. Search again after saving.'; };
			const revealTitle = () => {
				searchTitleReveal = true;
				const surface = $sourceMode ? source.view?.scrollDOM : editorElement?.closest('.editor-body');
				if (surface) surface.scrollTop = 0;
			};
			if (target.field !== 'body') {
				const value = target.field === 'title' ? $activeNote.meta.title : $activeNote.meta.tags.join(' ');
				if (!resolveSearchTarget(value, target)) { changed(); return; }
				if (target.field === 'title') revealTitle();
				else {
					searchTagsReveal = true;
					searchRevealNotice = `Matched tag: ${target.text}`;
				}
				return;
			}
			let body = $editorDirty ? getCurrentBody() : $activeNote.content;
			let match = resolveSearchTarget(body, target);
			if (!match) { changed(); return; }
			let visible = visibleSearchTarget(body, $activeNote.meta.title, $appConfig?.hide_title_in_body ?? false, match);
			if (visible.hiddenTitle) { revealTitle(); return; }
			if (!visible.match) { changed(); return; }
			if (!$sourceMode && editor) {
				// Search rendered text directly: never re-render the note just to locate a hit.
				const richMatch = findDocumentMatches(editor.state.doc, target.text, { matchCase: true, wholeWord: false })[0];
				if (richMatch) {
					const decorations = DecorationSet.create(editor.state.doc, [
						Decoration.inline(richMatch.from, richMatch.to, {
							class: 'note-search-match note-search-active',
							'data-search-passage': '',
						}),
					]);
					editor.view.dispatch(editor.state.tr.setMeta(targetRevealPluginKey, decorations));
					const element = editor.view.dom.querySelector<HTMLElement>('[data-search-passage]');
					if (element) {
						for (let parent = element.parentElement; parent; parent = parent.parentElement) {
							if (parent.matches('[data-type="details"]') && !parent.classList.contains('is-open')) {
								searchRevealedDetails.add(parent);
								openDetailsEl(parent);
							}
						}
						if (element.getClientRects().length) {
							element.scrollIntoView({ block: 'center', behavior: 'instant' });
							return;
						}
					}
				}
				// Destinations, attributes and other non-rendered Markdown need the source view.
				closeNoteSearch(false);
				mode = true;
				$sourceMode = true;
				await tick();
				await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
				if (!current()) return;
				source.load(stripTitleH1(body));
				searchRevealNotice = 'Match is in Markdown source.';
			}
			if (!current() || !source.view) return;
			if (visible.markdown !== source.state.doc.toString()) {
				// A previous mode conversion may have normalized Markdown without changing the note.
				body = getCurrentBody();
				match = resolveSearchTarget(body, target);
				if (!match) { changed(); return; }
				visible = visibleSearchTarget(body, $activeNote.meta.title, $appConfig?.hide_title_in_body ?? false, match);
			}
			if (visible.match) source.highlight(visible.match, true);
			else changed();
		} finally {
			if (request === targetRevealRequest) {
				if (current()) targetRevealTimer = setTimeout(clearTargetReveal, 4000);
				else clearTargetReveal();
			}
		}
	}

	function resolveTaskTarget(task: TaskRecord, content: string): TaskRevealTarget | null {
		const lines = content.split(/\r?\n/).map((line) => line.replace(/\r$/, ''));
		const rawLine = task.raw_line.replace(/\r$/, '');
		const exactLines = lines.flatMap((line, index) => line === rawLine ? [index] : []);
		const line = lines[task.line] === rawLine
			? task.line
			: exactLines.length === 1 ? exactLines[0] : null;
		if (line === null || !TASK_LINE_RE.test(lines[line])) return null;

		let sourceOrdinal = -1;
		let richOrdinal = -1;
		for (let index = 0; index <= line; index++) {
			if (TASK_LINE_RE.test(lines[index])) sourceOrdinal++;
			if (RICH_TASK_LINE_RE.test(lines[index])) richOrdinal++;
		}
		return {
			sourceOrdinal,
			richOrdinal: RICH_TASK_LINE_RE.test(lines[line]) ? richOrdinal : null,
		};
	}

	function sourceTaskRange(ordinal: number): { start: number; end: number } | null {
		let current = -1;
		let offset = 0;
		for (const line of source.state.doc.iterLines()) {
			if (TASK_LINE_RE.test(line) && ++current === ordinal) return { start: offset, end: offset + line.length };
			offset += line.length + 1;
		}
		return null;
	}

	function revealTaskTarget(target: TaskRevealTarget): boolean {
		clearTargetReveal();
		if ($sourceMode) {
			if (!source.view) return false;
			const range = sourceTaskRange(target.sourceOrdinal);
			if (!range) return true;
			source.highlight({ from: range.start, to: range.end }, true);
			const origin = captureOrigin();
			targetRevealTimer = setTimeout(() => {
				if (isOriginCurrent(origin) && !noteSearchOpen) source.highlight(null);
				targetRevealTimer = null;
			}, 1600);
			return true;
		}

		if (!editor || target.richOrdinal === null) return !!editor;
		let current = -1;
		let targetPos: number | null = null;
		editor.state.doc.descendants((node, pos) => {
			if (targetPos !== null || node.type.name !== 'taskItem') return targetPos === null;
			current++;
			if (current === target.richOrdinal) targetPos = pos;
			return targetPos === null;
		});
		if (targetPos === null) return true;
		const node = editor.state.doc.nodeAt(targetPos);
		if (!node) return true;
		const decorations = DecorationSet.create(editor.state.doc, [
			Decoration.node(targetPos, targetPos + node.nodeSize, { class: 'task-target-reveal' }),
		]);
		editor.view.dispatch(editor.state.tr.setMeta(targetRevealPluginKey, decorations));
		const element = editor.view.nodeDOM(targetPos);
		if (!(element instanceof HTMLElement)) return true;
		const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
		element.scrollIntoView({ block: 'center', behavior: reduceMotion ? 'auto' : 'smooth' });
		const decoratedEditor = editor;
		targetRevealTimer = setTimeout(() => {
			if (!decoratedEditor.isDestroyed) {
				decoratedEditor.view.dispatch(decoratedEditor.state.tr.setMeta(targetRevealPluginKey, DecorationSet.empty));
			}
			targetRevealTimer = null;
		}, 2200);
		return true;
	}

	function scheduleTaskReveal(path: string, target: TaskRevealTarget | null, request: number, attempts = 0) {
		if (!target) return;
		tick().then(() => requestAnimationFrame(() => {
			if (request !== targetRevealRequest || path !== loadedPath) return;
			if (!revealTaskTarget(target) && attempts < 3) {
				scheduleTaskReveal(path, target, request, attempts + 1);
			}
		}));
	}


	function closeAllDropdowns() {
		closeFormattingDropdowns();
		closeMore();
		formattingTrigger = null;
		tablePickerHover = { rows: 0, cols: 0 };
	}

	let anyDropdownOpen = $derived(headingDropdown || colorDropdown || highlightDropdown || alignDropdown || insertDropdown || tablePickerOpen);

	let editorState = $state(0);
	let editorStateRaf = 0; // RAF handle for batching toolbar updates

	function isEditorActive(
		nameOrAttributes: string | Record<string, unknown>,
		attributes?: Record<string, unknown>,
	): boolean {
		void editorState;
		if (!editor) return false;
		return typeof nameOrAttributes === 'string'
			? editor.isActive(nameOrAttributes, attributes)
			: editor.isActive(nameOrAttributes);
	}

	function getEditorAttributes(name: string): Record<string, unknown> {
		void editorState;
		return editor?.getAttributes(name) ?? {};
	}

	// AI
	let aiMenu = $state<{ x: number; y: number } | null>(null);
	let aiLoading = $state(false);
	let aiResult = $state<string | null>(null);
	let aiError = $state<string | null>(null);
	let aiSelectionFrom = $state(0);
	let aiSelectionTo = $state(0);
	let aiSelectedText = $state('');
	let aiCustomPrompt = $state('');
	let aiShowCustom = $state(false);
	let aiTranslateMenu = $state(false);
	let aiWholeNote = $state(false);
	let aiEmptyNote = $state(false);
	let aiOriginalMarkdown = $state('');
	let aiMediaPlaceholders = $state<Map<string, string>>(new Map());
	let aiStreamUnlisten: (() => void) | null = null;
	let aiOrigin: EditorOrigin | null = null;
	let aiGeneration = 0;


	// Outline entries are mapped immediately; labels are rescanned after typing settles.
	let showOutline = $state(false);
	let outlineHeadings = $state<OutlineHeading[]>([]);
	let outlineOrigin: EditorOrigin | null = null;
	let outlineActive = $state(-1);
	let outlineList = $state<HTMLUListElement>();
	let outlineTrigger: HTMLElement | null = null;
	let outlineTimer: ReturnType<typeof setTimeout> | null = null;
	let outlineFrame: number | null = null;

	function clearOutline() {
		if (outlineTimer !== null) clearTimeout(outlineTimer);
		if (outlineFrame !== null) cancelAnimationFrame(outlineFrame);
		outlineTimer = null;
		outlineFrame = null;
		outlineHeadings = [];
		outlineOrigin = null;
		outlineActive = -1;
	}

	function updateOutline() {
		if (outlineTimer !== null) clearTimeout(outlineTimer);
		outlineTimer = null;
		if (editorDisposed || !loadedPath || loadedPath !== $activeNotePath) { clearOutline(); return; }
		outlineHeadings = $sourceMode ? sourceHeadings(source.state)
			: editor && pendingContent === null ? documentHeadings(editor.state.doc) : [];
		outlineOrigin = captureOrigin();
		scheduleOutlineActive();
	}

	function scheduleOutline() {
		if (outlineTimer !== null) clearTimeout(outlineTimer);
		outlineTimer = setTimeout(() => {
			outlineTimer = null;
			if (showOutline && !editorDisposed) updateOutline();
		}, 250);
	}

	function updateOutlineActive() {
		if (!outlineOrigin || !isOriginCurrent(outlineOrigin)) return;
		if ($sourceMode) {
			const view = source.view;
			if (!view || !view.scrollDOM.getClientRects().length) return;
			const height = Math.max(0, view.scrollDOM.getBoundingClientRect().top + 24 - view.documentTop);
			outlineActive = activeHeadingIndex(outlineHeadings, view.lineBlockAtHeight(height).to);
		} else if (editor) {
			const body = editor.view.dom.closest('.editor-body');
			if (!body || !body.getClientRects().length) return;
			const readingTop = body.getBoundingClientRect().top + 32;
			let active = -1;
			for (let i = 0; i < outlineHeadings.length; i++) {
				const element = editor.view.nodeDOM(outlineHeadings[i].pos);
				if (!(element instanceof HTMLElement) || !element.getClientRects().length) continue;
				if (element.getBoundingClientRect().top > readingTop) break;
				active = i;
			}
			outlineActive = active;
		}
	}

	function scheduleOutlineActive() {
		if (!showOutline || outlineFrame !== null) return;
		outlineFrame = requestAnimationFrame(() => {
			outlineFrame = null;
			updateOutlineActive();
		});
	}

	$effect(() => { onOutlineChange?.(showOutline && !showHistory, outlinePanelId); });
	$effect(() => {
		const active = outlineActive;
		if (!showOutline || active < 0) return;
		tick().then(() => {
			if (!outlineList || outlineList.contains(document.activeElement)) return;
			const item = outlineList.children[active] as HTMLElement | undefined;
			if (!item) return;
			const listRect = outlineList.getBoundingClientRect();
			const itemRect = item.getBoundingClientRect();
			if (itemRect.top < listRect.top) outlineList.scrollTop -= listRect.top - itemRect.top;
			else if (itemRect.bottom > listRect.bottom) outlineList.scrollTop += itemRect.bottom - listRect.bottom;
		});
	});

	function handleOutlineResize(delta: number) {
		$outlineWidth = Math.max(160, Math.min(500, $outlineWidth - delta));
	}

	async function closeOutline() {
		showOutline = false;
		await tick();
		if (outlineTrigger?.isConnected) outlineTrigger.focus({ preventScroll: true });
	}

	async function scrollToHeading(heading: OutlineHeading) {
		if (!outlineOrigin || !isOriginCurrent(outlineOrigin)) { updateOutline(); return; }
		// A mapped position is not enough: the heading may have been converted or deleted.
		const current = $sourceMode ? sourceHeadings(source.state) : editor ? documentHeadings(editor.state.doc) : [];
		const target = current.find(h => h.pos === heading.pos);
		if (!target) { updateOutline(); return; }
		const origin = captureOrigin();
		if (isMobile) {
			showOutline = false;
			await tick();
		}
		if (!isOriginCurrent(origin, true)) return;
		if ($sourceMode) {
			source.view?.requestMeasure();
			source.select(target.pos, target.pos, true);
			source.focus();
		} else if (editor) {
			const element = editor.view.nodeDOM(target.pos);
			if (element instanceof HTMLElement) {
				for (let parent = element.parentElement; parent && parent !== editor.view.dom; parent = parent.parentElement) {
					if (parent.matches('[data-type="details"]')) openDetailsEl(parent);
					else if (parent.matches('.callout.is-folded')) {
						// Reveal without changing the saved fold state; the node view keeps it open while it lives.
						parent.setAttribute('data-revealed', '');
						parent.classList.remove('is-folded');
					}
				}
			}
			editor.commands.setTextSelection(target.pos + 1);
			editor.commands.scrollIntoView();
			editor.view.focus();
			if (element instanceof HTMLElement) element.scrollIntoView({ block: 'start' });
		}
		outlineActive = current.indexOf(target);
	}

	// Version history
	let showHistory = $state(false);
	let showGraph = $state(false);
	let tagMenu = $state<{ x: number; y: number } | null>(null);
	let historyVersions = $state<VersionEntry[]>([]);
	let historyPreview = $state<string | null>(null);
	let historySelected = $state<VersionEntry | null>(null);
	let historyLoading = $state(false);
	let historyError = $state<string | null>(null);
	let historyPreviewLoading = $state(false);
	let historyPreviewError = $state<string | null>(null);
	let historyWorking = $state('');

	// Info panel
	let showInfo = $state(false);
	let wordCount = $state(0);
	let charCount = $state(0);
	let infoPathCopyState = $state<'idle' | 'copied' | 'error'>('idle');
	let infoPathCopyTimer: ReturnType<typeof setTimeout> | null = null;
	const infoFrontmatter = $derived(showInfo && !showHistory && $activeNote
		? splitSnapshot($activeNote.raw).metadata
		: '');

	$effect(() => {
		$activeNotePath;
		infoPathCopyState = 'idle';
		if (infoPathCopyTimer) clearTimeout(infoPathCopyTimer);
		infoPathCopyTimer = null;
	});

	// In-note search
	let noteSearchOpen = $state(false);
	let noteSearchQuery = $state('');
	let noteSearchIndex = $state(0);
	let noteSearchResults = $state.raw<NoteSearchMatch[]>([]);
	let noteSearchMatchCase = $state(false);
	let noteSearchWholeWord = $state(false);
	let noteSearchPending = $state(false);
	let noteSearchInput = $state<HTMLInputElement>(null!);
	let noteReplaceOpen = $state(false);
	let noteReplaceValue = $state('');
	let noteReplaceInput = $state<HTMLInputElement>(null!);
	const noteSearchPluginKey = new PluginKey('noteSearch');

	// Slash commands
	let slashMenu = $state<{ x: number; y: number; query: string; from: number; to: number } | null>(null);
	let slashSelectedIndex = $state(0);
	let slashTablePicker = $state(false);
	let slashTableHover = $state({ rows: 0, cols: 0 });
	let slashColorPicker = $state(false);
	let slashColorHex = $state('#4b6abf');
	let slashColorInputEl = $state<HTMLInputElement | null>(null);
	const colorPresets = ['#ef4444', '#f97316', '#eab308', '#22c55e', '#06b6d4', '#3b82f6', '#6366f1', '#a855f7', '#ec4899', '#64748b', '#000000', '#ffffff'];

	interface SlashCommand {
		label: string;
		aliases: string[];
		icon: string;
		action: () => void;
	}

	function insertTimestamp(kind: 'date' | 'time' | 'datetime') {
		if (!editor) return;
		const now = new Date();
		const pad = (n: number) => String(n).padStart(2, '0');
		const date = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
		const time = formatClockTime(now, $appConfig?.time_format);
		const text = kind === 'date' ? date : kind === 'time' ? time : `${date} ${time}`;
		editor.chain().focus().insertContent(text).run();
	}

	function getSlashCommands(): SlashCommand[] {
		return [
			{ label: 'Heading 1', aliases: ['h1', 'heading1', 'title'], icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M4 12h8M4 4v16M12 4v16M17 12l3-2v8"/></svg>', action: () => editor?.chain().focus().toggleHeading({ level: 1 }).run() },
			{ label: 'Heading 2', aliases: ['h2', 'heading2', 'subtitle'], icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M4 12h8M4 4v16M12 4v16"/><path d="M21 18h-4c0-4 4-3 4-6 0-1.5-2-2.5-4-1"/></svg>', action: () => editor?.chain().focus().toggleHeading({ level: 2 }).run() },
			{ label: 'Heading 3', aliases: ['h3', 'heading3'], icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M4 12h8M4 4v16M12 4v16"/><path d="M17.5 10.5c1.7-1 3.5 0 3.5 1.5a2 2 0 01-2 2m2 0a2 2 0 01-2 2c-1.5 0-3.5 0-3.5-1.5"/></svg>', action: () => editor?.chain().focus().toggleHeading({ level: 3 }).run() },
			{ label: 'Bullet List', aliases: ['ul', 'unordered', 'bullets', 'list'], icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="8" y1="6" x2="21" y2="6"/><line x1="8" y1="12" x2="21" y2="12"/><line x1="8" y1="18" x2="21" y2="18"/><circle cx="3" cy="6" r="1" fill="currentColor"/><circle cx="3" cy="12" r="1" fill="currentColor"/><circle cx="3" cy="18" r="1" fill="currentColor"/></svg>', action: toggleBulletList },
			{ label: 'Numbered List', aliases: ['ol', 'ordered', 'number'], icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="10" y1="6" x2="21" y2="6"/><line x1="10" y1="12" x2="21" y2="12"/><line x1="10" y1="18" x2="21" y2="18"/><text x="1" y="9" font-size="8" fill="currentColor" stroke="none">1</text><text x="1" y="15" font-size="8" fill="currentColor" stroke="none">2</text><text x="1" y="21" font-size="8" fill="currentColor" stroke="none">3</text></svg>', action: () => editor?.chain().focus().toggleOrderedList().run() },
			{ label: 'Task List', aliases: ['checklist', 'checkbox', 'todo', 'check'], icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="5" width="6" height="6" rx="1"/><path d="M5 8l1.5 1.5L9 7"/><line x1="13" y1="8" x2="21" y2="8"/><rect x="3" y="14" width="6" height="6" rx="1"/><line x1="13" y1="17" x2="21" y2="17"/></svg>', action: toggleTaskList },
			{ label: 'Code Block', aliases: ['code', 'codeblock', 'pre', 'snippet'], icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/></svg>', action: () => editor?.chain().focus().toggleCodeBlock().run() },
			{ label: 'Secret', aliases: ['secret', 'encrypt', 'password', 'private'], icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="11" width="18" height="10" rx="2"/><path d="M7 11V7a5 5 0 0110 0v4"/></svg>', action: () => openSecretInsert() },
			{ label: 'Blockquote', aliases: ['quote', 'blockquote', 'citation'], icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 21c3 0 7-1 7-8V5c0-1.25-.756-2.017-2-2H4c-1.25 0-2 .75-2 1.972V11c0 1.25.75 2 2 2 1 0 1 0 1 1v1c0 1-1 2-2 2s-1 .008-1 1.031V20c0 1 0 1 1 1z"/><path d="M15 21c3 0 7-1 7-8V5c0-1.25-.757-2.017-2-2h-4c-1.25 0-2 .75-2 1.972V11c0 1.25.75 2 2 2h.75c0 2.25.25 4-2.75 4v3c0 1 0 1 1 1z"/></svg>', action: () => editor?.chain().focus().toggleBlockquote().run() },
			{ label: 'Collapsible Section', aliases: ['details', 'accordion', 'collapse', 'toggle', 'summary'], icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2"/><polyline points="10 8 14 12 10 16"/></svg>', action: () => insertDetails() },
			{ label: 'Callout', aliases: ['callout', 'admonition', 'note', 'info', 'tip', 'warning', 'caution', 'danger', 'success', 'question', 'quote', 'aside', 'box'], icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="5" width="18" height="14" rx="2"/><line x1="7" y1="5" x2="7" y2="19"/></svg>', action: () => insertCallout('note') },
			{ label: 'Table', aliases: ['table', 'grid'], icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="3" width="18" height="18" rx="2"/><line x1="3" y1="9" x2="21" y2="9"/><line x1="3" y1="15" x2="21" y2="15"/><line x1="9" y1="3" x2="9" y2="21"/><line x1="15" y1="3" x2="15" y2="21"/></svg>', action: () => { slashTablePicker = true; slashTableHover = { rows: 0, cols: 0 }; } },
			{ label: 'Image Grid', aliases: ['imagegrid', 'image-grid', 'imggrid', 'gallery', 'masonry', 'images', 'side-by-side'], icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/></svg>', action: () => openAssetPicker('insert-grid-input') },
			{ label: 'Horizontal Rule', aliases: ['hr', 'divider', 'line', 'separator', 'rule'], icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="2" y1="12" x2="22" y2="12"/></svg>', action: () => editor?.chain().focus().setHorizontalRule().run() },
			{ label: 'Page Break', aliases: ['pagebreak', 'page', 'break', 'newpage', 'print'], icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><line x1="2" y1="9" x2="22" y2="9" stroke-dasharray="4 2"/><line x1="2" y1="15" x2="22" y2="15" stroke-dasharray="4 2"/><path d="M6 5v4M18 5v4M6 15v4M18 15v4"/></svg>', action: () => editor?.chain().focus().insertContent({ type: 'pageBreak' }).run() },
			{ label: 'Math Block', aliases: ['math', 'latex', 'equation', 'formula', 'tex', 'katex'], icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 5h6l4 14h6"/><path d="M7 19l10-14"/></svg>', action: () => openMathInsert('block') },
			{ label: 'Math Inline', aliases: ['mathinline', 'inline-math', 'imath', 'inlinemath'], icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 8h2l3 8h2"/><path d="M8 12l8-4"/></svg>', action: () => openMathInsert('inline') },
			{ label: 'Date', aliases: ['date', 'today', 'day'], icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="18" rx="2"/><line x1="16" y1="2" x2="16" y2="6"/><line x1="8" y1="2" x2="8" y2="6"/><line x1="3" y1="10" x2="21" y2="10"/></svg>', action: () => insertTimestamp('date') },
			{ label: 'Time', aliases: ['time', 'clock'], icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/></svg>', action: () => insertTimestamp('time') },
			{ label: 'Date & Time', aliases: ['datetime', 'now', 'timestamp', 'stamp'], icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 7.5V6a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2h6"/><line x1="3" y1="10" x2="21" y2="10"/><line x1="8" y1="2" x2="8" y2="6"/><line x1="16" y1="2" x2="16" y2="6"/><circle cx="18" cy="17" r="4"/><path d="M18 15.5v1.5l1 1"/></svg>', action: () => insertTimestamp('datetime') },
			{ label: 'Color', aliases: ['color', 'colour', 'hex', 'rgb', 'swatch', 'palette'], icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="13.5" cy="6.5" r=".5" fill="currentColor"/><circle cx="17.5" cy="10.5" r=".5" fill="currentColor"/><circle cx="8.5" cy="7.5" r=".5" fill="currentColor"/><circle cx="6.5" cy="12.5" r=".5" fill="currentColor"/><path d="M12 2C6.5 2 2 6.5 2 12s4.5 10 10 10c.926 0 1.648-.746 1.648-1.688 0-.437-.18-.835-.437-1.125-.29-.289-.438-.652-.438-1.125a1.64 1.64 0 0 1 1.668-1.668h1.996c3.051 0 5.555-2.503 5.555-5.554C21.965 6.012 17.461 2 12 2z"/></svg>', action: () => { slashColorPicker = true; } },
		];
	}

	let slashFiltered = $derived.by(() => {
		const commands = getSlashCommands();
		if (!slashMenu) return commands;
		const q = slashMenu.query.toLowerCase();
		if (!q) return commands;
		return commands.filter(cmd =>
			cmd.label.toLowerCase().includes(q) ||
			cmd.aliases.some(a => a.includes(q))
		);
	});
	let titleInput = $state<HTMLInputElement>(null!);
	let linkModal = $state(false);
	let linkModalUrl = $state('');
	let linkModalInput = $state<HTMLInputElement>(null!);
	let linkSelectionFrom = 0;
	let linkSelectionTo = 0;
	let linkModalDisplayText = '';
	let linkSuggestIndex = $state(0);
	let linkSuggestTitles = $state<NoteTitleEntry[]>([]);
	let linkSuggestFiltered = $derived.by(() => {
		const q = linkModalUrl.trim().toLowerCase();
		if (!q || q.startsWith('http://') || q.startsWith('https://') || q.startsWith('mailto:')) return [];
		return linkSuggestTitles.filter(e => e.title.toLowerCase().includes(q)).slice(0, 8);
	});
	let textContextMenu = $state<{ x: number; y: number; submenuLeft: boolean } | null>(null);
	let tableContextMenu = $state<{ x: number; y: number; hasStyling: boolean } | null>(null);
	let imageToolbar = $state<{ pos: number; x: number; y: number; size: string; src: string; alt: string; title: string; inGrid: boolean } | null>(null);
	let imageTitleModal = $state<{ title: string } | null>(null);
	let imageViewer = $state<{ src: string; alt: string } | null>(null);
	let copyToast = $state<'copying' | 'done' | null>(null);

	// Math insert/edit modal (opened by /math slash command or double-click on existing math node)
	let mathModal = $state<{ kind: 'block' | 'inline'; editPos: number | null; tex: string } | null>(null);

	function openMathInsert(kind: 'block' | 'inline') {
		mathModal = { kind, editPos: null, tex: '' };
	}
	function openMathEdit(pos: number, kind: 'block' | 'inline', tex: string) {
		mathModal = { kind, editPos: pos, tex };
	}
	function cancelMathModal() {
		mathModal = null;
	}
	function commitMathModal() {
		if (!editor || !mathModal) return;
		const { kind, editPos, tex } = mathModal;
		const trimmed = tex.trim();
		mathModal = null;
		if (!trimmed) return;
		if (editPos !== null) {
			const node = editor.state.doc.nodeAt(editPos);
			if (node && (node.type.name === 'mathBlock' || node.type.name === 'mathInline')) {
				const tr = editor.state.tr.setNodeAttribute(editPos, 'tex', trimmed);
				editor.view.dispatch(tr);
			}
		} else {
			const nodeType = kind === 'block' ? 'mathBlock' : 'mathInline';
			editor.chain().focus().insertContent({ type: nodeType, attrs: { tex: trimmed } }).run();
		}
	}

	type SecretModalState = {
		title: string;
		text: string;
		passphrase: string;
		confirmPassphrase: string;
		from: number;
		to: number;
		error: string;
		busy: boolean;
	};

	let secretModal = $state<SecretModalState | null>(null);
	let secretTitleInput = $state<HTMLInputElement | null>(null);
	let secretOrigin: EditorOrigin | null = null;

	function selectedPlainText(): string {
		if (!editor) return '';
		const { from, to } = editor.state.selection;
		return from === to ? '' : editor.state.doc.textBetween(from, to, '\n\n');
	}

	function openSecretInsert() {
		const selection = editor?.state.selection;
		const origin = captureOrigin();
		secretOrigin = origin;
		secretModal = {
			title: 'Encrypted secret',
			text: selectedPlainText(),
			passphrase: '',
			confirmPassphrase: '',
			from: selection?.from ?? 0,
			to: selection?.to ?? 0,
			error: '',
			busy: false,
		};
		tick().then(() => {
			if (!secretModal || secretOrigin !== origin || !isOriginCurrent(origin)) return;
			secretTitleInput?.focus();
			secretTitleInput?.select();
		});
	}

	function cancelSecretModal() {
		secretModal = null;
	}

	async function commitSecretModal() {
		if (!editor || !secretModal || secretModal.busy) return;
		const modal = secretModal;
		const origin = secretOrigin;
		if (!origin || !canInsert(origin) || !isOriginCurrent(origin, true)) return;
		if (!secretModal.text) {
			secretModal.error = 'Secret text is required.';
			return;
		}
		if (!secretModal.passphrase) {
			secretModal.error = 'Passphrase is required.';
			return;
		}
		if (secretModal.passphrase !== secretModal.confirmPassphrase) {
			secretModal.error = 'Passphrases do not match.';
			return;
		}
		secretModal.busy = true;
		secretModal.error = '';
		try {
			const payload = await encryptSecretText(modal.text, modal.passphrase, modal.title);
			if (!canInsert(origin) || !isOriginCurrent(origin, true) || secretModal !== modal || !editor) return;
			const { from, to } = modal;
			secretModal = null;
			editor.commands.insertContentAt({ from, to }, { type: 'secretBlock', attrs: { payload } });
		} catch (e: any) {
			if (secretModal === modal && isOriginCurrent(origin)) {
				secretModal.error = e?.message || 'Could not encrypt secret.';
				secretModal.busy = false;
			}
		}
	}
	const katexCache = new Map<string, string>();
	function renderKatex(tex: string, displayMode: boolean): string {
		const key = (displayMode ? 'B:' : 'I:') + tex;
		let html = katexCache.get(key);
		if (html === undefined) {
			try {
				html = katex.renderToString(tex, { displayMode, throwOnError: false });
			} catch {
				html = `<span class="katex-error">${escapeHtml(tex)}</span>`;
			}
			katexCache.set(key, html);
			if (katexCache.size > 512) katexCache.delete(katexCache.keys().next().value!);
		}
		return html;
	}

	function mathHTML(tex: string, displayMode: boolean): HTMLElement {
		const dom = document.createElement(displayMode ? 'div' : 'span');
		dom.setAttribute(displayMode ? 'data-math-block' : 'data-math-inline', encodeURIComponent(tex));
		dom.className = displayMode ? 'math-block' : 'math-inline';
		dom.contentEditable = 'false';
		dom.innerHTML = renderKatex(tex, displayMode);
		return dom;
	}

	let mathObserver: IntersectionObserver | null = null;
	const mathPending = new WeakMap<Element, () => void>();
	function observeMath(dom: HTMLElement, render: () => void) {
		if (!mathObserver) {
			const root = (editorElement?.closest('.editor-body') as Element) ?? null;
			mathObserver = new IntersectionObserver((entries) => {
				for (const e of entries) {
					if (!e.isIntersecting) continue;
					const fn = mathPending.get(e.target);
					if (fn) { mathPending.delete(e.target); mathObserver!.unobserve(e.target); fn(); }
				}
			}, { root, rootMargin: '1000px 0px' });
		}
		mathPending.set(dom, render);
		mathObserver.observe(dom);
	}

	function renderMathPreview(tex: string, displayMode: boolean): string {
		if (!tex.trim()) return '';
		try {
			return katex.renderToString(tex, { displayMode, throwOnError: false });
		} catch (e: any) {
			return `<span class="math-modal-preview-error">${e?.message || String(e)}</span>`;
		}
	}

	// External-file viewer mode UI
	let viewerImportPickerOpen = $state(false);
	let viewerImportBusy = $state(false);
	let viewerToast = $state<string | null>(null);
	let viewerFlatNotebooks = $derived.by(() => {
		if (!viewerImportPickerOpen) return [] as Array<{ path: string; name: string; depth: number }>;
		const out: Array<{ path: string; name: string; depth: number }> = [];
		const walk = (list: any[], depth: number) => {
			for (const nb of list) {
				out.push({ path: nb.relative_path.replace(/\\/g, '/'), name: nb.name, depth });
				if (nb.children?.length) walk(nb.children, depth + 1);
			}
		};
		walk($notebooks, 0);
		return out;
	});

	function viewerFlash(msg: string) {
		viewerToast = msg;
		setTimeout(() => { viewerToast = null; }, 1500);
	}

	function closeViewer() {
		beginEditorNoteNavigation();
		$viewerNote = null;
		$activeNote = null;
		$activeNotePath = null;
		$readOnly = false;
		$focusMode = false;
	}

	async function viewerImportTo(folderPath: string) {
		const isCurrent = beginEditorNoteNavigation();
		const v = $viewerNote;
		const vaultRoot = $appConfig?.active_vault;
		if (!isCurrent() || !v || !vaultRoot || viewerImportBusy) return;
		viewerImportBusy = true;
		try {
			const raw = $activeNote?.raw;
			if (raw === undefined || $activeNote?.path !== v.path) throw new Error('Reopen the source note before importing.');
			const content = await importNote(vaultRoot, v.path, folderPath || null, raw, externalNoteReferences(raw));
			if (!isCurrent()) return;
			const dest = content.path;
			// Switch to the imported note as a real vault note
			$viewerNote = null;
			$readOnly = false;
			$focusMode = false;
			viewerImportPickerOpen = false;
			$activeNote = content;
			$activeNotePath = dest;
			$editorDirty = false;
			loadNote(dest, content.content);
			viewerFlash('Imported');
		} catch (e: any) {
			if (!isCurrent()) return;
			console.error('[Viewer] import failed', e);
			viewerFlash('Import failed: ' + (e?.message || String(e)));
		} finally {
			viewerImportBusy = false;
		}
	}
	// Normalize Windows backslashes to '/' so the prefix strip and folder split work cross-platform (issue #99).
	let noteRelativePath = $derived($activeNotePath && $appConfig?.active_vault ? $activeNotePath.replace(/\\/g, '/').replace($appConfig.active_vault.replace(/\\/g, '/') + '/', '') : '');
	let noteFolder = $derived(noteRelativePath ? noteRelativePath.substring(0, noteRelativePath.lastIndexOf('/')) : '');
	let isQuickAccess = $derived(noteRelativePath ? $quickAccessPaths.includes(noteRelativePath) : false);

	const lowlight = createLowlight(common);
	lowlight.register('powershell', powershell);
	const codeLanguages = [...lowlight.listLanguages(), 'mermaid'].sort();
	const mdit = createEditorMarkdownParser();

	function normalizePath(p: string): string {
		const parts = p.split('/');
		const resolved: string[] = [];
		for (const seg of parts) {
			if (seg === '..') {
				resolved.pop();
			} else if (seg !== '.') {
				resolved.push(seg);
			}
		}
		return resolved.join('/');
	}

	/**
	 * Resolve a note path (which may be vault-relative or absolute) to a full
	 * filesystem path for file I/O operations (readNote, renameNote, etc.).
	 *
	 * Wikilink data-path attributes store vault-relative paths (e.g. "folder/note.md")
	 * for cross-platform portability. This helper joins them with the active vault
	 * root so the Rust backend receives the full path it expects.
	 *
	 * Legacy absolute paths (from older Windows builds) are passed through as-is
	 * so they don't break before the migration pass rewrites them.
	 */
	function resolveNotePath(path: string): string {
		if (!path) return path;
		const vaultRoot = $appConfig?.active_vault;
		if (!vaultRoot) return path;
		// Normalize separators for comparison
		const norm = path.replace(/\\/g, '/');
		const root = vaultRoot.replace(/\\/g, '/');
		// Already absolute (starts with vault root, or has a Windows drive letter, or starts with /)
		if (norm.startsWith(root + '/')) return path;
		if (/^[A-Za-z]:[\\/]/.test(path)) return path;
		if (path.startsWith('/')) return path;
		// Relative: join with vault root
		return vaultRoot + '/' + path;
	}

	/**
	 * Normalize a data-path value to a vault-relative path with forward slashes.
	 * Handles legacy absolute paths (from older Windows builds) and Windows
	 * backslashes so wikilinks survive cross-device sync.
	 */
	function normalizeWikiPath(path: string): string {
		if (!path) return '';
		const vaultRoot = $appConfig?.active_vault;
		let norm = path.replace(/\\/g, '/');
		if (vaultRoot) {
			const root = vaultRoot.replace(/\\/g, '/');
			if (norm.startsWith(root + '/')) {
				norm = norm.slice(root.length + 1);
			}
		}
		return norm;
	}

	const CustomImage = Image.extend({
		addAttributes() {
			return {
				...this.parent?.(),
				src: {
					default: null,
					parseHTML: (element: HTMLElement) => resolveImageSrc(element.getAttribute('src') ?? ''),
				},
				size: {
					default: 'full',
					parseHTML: (element: HTMLElement) => element.getAttribute('data-size') || 'full',
					renderHTML: (attributes: Record<string, any>) => {
						return { 'data-size': attributes.size };
					},
				},
			};
		},
	});

	function cellColorAttributes() {
		return {
			backgroundColor: {
				default: null,
				parseHTML: (element: HTMLElement) => element.getAttribute('data-bg-color') || element.style.backgroundColor || null,
				renderHTML: (attributes: Record<string, any>) => {
					if (!attributes.backgroundColor) return {};
					const bg = attributes.backgroundColor;
					// Determine if we need light text for dark backgrounds
					const darkBgs = ['#1e293b', '#374151', '#7f1d1d', '#713f12', '#14532d', '#1e3a5f', '#4c1d95', '#831843', '#0c4a6e', '#064e3b'];
					const needsLight = darkBgs.includes(bg);
					const style = needsLight
						? `background-color: ${bg}; color: #f1f5f9`
						: `background-color: ${bg}`;
					return { style, 'data-bg-color': bg };
				},
			},
		};
	}

	/** Column alignment from a Markdown table's delimiter row (`:---`, `:---:`, `---:`), kept per cell. */
	function cellAlignAttributes() {
		return {
			align: {
				default: null,
				parseHTML: (element: HTMLElement) => {
					const align = element.style.textAlign || element.getAttribute('align') || '';
					return align === 'left' || align === 'center' || align === 'right' ? align : null;
				},
				renderHTML: (attributes: Record<string, any>) => attributes.align ? { style: `text-align: ${attributes.align}` } : {},
			},
		};
	}

	const CustomTableCell = TableCell.extend({
		...keepCodeWhitespace,
		addAttributes() {
			return { ...this.parent?.(), ...cellColorAttributes(), ...cellAlignAttributes() };
		},
	});

	const CustomTableHeader = TableHeader.extend({
		...keepCodeWhitespace,
		addAttributes() {
			return { ...this.parent?.(), ...cellColorAttributes(), ...cellAlignAttributes() };
		},
	});

	const PdfEmbed = TiptapNode.create({
		name: 'pdfEmbed',
		group: 'block',
		atom: true,
		addAttributes() {
			return {
				src: { default: null },
				name: { default: 'file.pdf' },
			};
		},
		parseHTML() {
			return [{
				tag: 'div[data-pdf-src]',
				getAttrs: (el: HTMLElement) => ({
					src: el.getAttribute('data-pdf-src'),
					name: el.getAttribute('data-pdf-name') || 'file.pdf',
				}),
			}];
		},
		renderHTML({ HTMLAttributes }) {
			const src = HTMLAttributes.src || '';
			const name = HTMLAttributes.name || 'file.pdf';
			const showInline = !isMobile && ($appConfig?.pdf_preview ?? false);
			if (showInline) {
				const root = currentNoteAssetRoot();
				const pdfHeight = $appConfig?.pdf_height ?? 600;
				const local = resolveFileReference(src, root ? `${root}/.vault-root` : $activeNotePath, root);
				const displaySrc = convertFileSrc(local.path) + local.suffix;
				return ['div', mergeAttributes({ 'data-pdf-src': src, 'data-pdf-name': name, class: 'pdf-embed' }),
					['iframe', { src: displaySrc, width: '100%', height: `${pdfHeight}px`, frameborder: '0' }],
					['p', { class: 'pdf-label' }, name],
				];
			}
			// Non-inline: render as a clickable link (mobile + desktop with setting off)
			return ['div', mergeAttributes({ 'data-pdf-src': src, 'data-pdf-name': name, class: 'pdf-embed-mobile' }),
				['a', { href: src, class: 'pdf-link-mobile' },
					['span', { class: 'pdf-icon-mobile' }, '\uD83D\uDCC4'],
					['span', {}, name],
				],
			];
		},
	});

	function renderSecretNode(dom: HTMLElement, payload: string) {
		dom.textContent = '';
		const secretTitle = readSecretTitle(payload);

		const header = document.createElement('div');
		header.className = 'secret-block-header';

		const title = document.createElement('span');
		title.className = 'secret-block-title';
		title.innerHTML = '<svg class="secret-block-lock" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="11" width="18" height="10" rx="2"/><path d="M7 11V7a5 5 0 0110 0v4"/></svg>';
		title.append(document.createTextNode(secretTitle));

		const badge = document.createElement('span');
		badge.className = 'secret-block-badge';
		badge.textContent = 'AES-GCM';

		header.append(title, badge);

		const form = document.createElement('form');
		form.className = 'secret-block-form';

		const input = document.createElement('input');
		input.type = 'password';
		input.placeholder = 'Passphrase';
		input.autocomplete = 'current-password';
		input.size = 1;

		const unlock = document.createElement('button');
		unlock.type = 'submit';
		unlock.textContent = 'Unlock';

		const error = document.createElement('div');
		error.className = 'secret-block-error';

		form.append(input, unlock);
		dom.append(header, form, error);

		form.addEventListener('submit', async (event) => {
			event.preventDefault();
			const origin = captureOrigin();
			error.textContent = '';
			unlock.disabled = true;
			try {
				const plaintext = await decryptSecretText(payload, input.value);
				if (!isOriginCurrent(origin) || !form.isConnected) return;
				renderUnlockedSecretNode(dom, payload, plaintext);
			} catch (e: any) {
				error.textContent = e?.message || 'Unable to unlock secret.';
				unlock.disabled = false;
			}
		});
	}

	function renderUnlockedSecretNode(dom: HTMLElement, payload: string, plaintext: string) {
		dom.textContent = '';
		const secretTitle = readSecretTitle(payload);

		const header = document.createElement('div');
		header.className = 'secret-block-header';

		const title = document.createElement('span');
		title.className = 'secret-block-title';
		title.textContent = secretTitle;

		const actions = document.createElement('div');
		actions.className = 'secret-block-actions';

		const copy = document.createElement('button');
		copy.type = 'button';
		copy.textContent = 'Copy';
		copy.addEventListener('click', async () => {
			await navigator.clipboard.writeText(plaintext);
			copy.textContent = 'Copied';
			setTimeout(() => { copy.textContent = 'Copy'; }, 1200);
		});

		const lock = document.createElement('button');
		lock.type = 'button';
		lock.textContent = 'Lock';
		lock.addEventListener('click', () => renderSecretNode(dom, payload));

		actions.append(copy, lock);
		header.append(title, actions);

		const body = document.createElement('pre');
		body.className = 'secret-block-plaintext';
		body.textContent = plaintext;

		dom.append(header, body);
	}

	/** Percent-decoding of an attribute from note content: a stray `%` must not abort loading the note. */
	function decodeNoteAttribute(value: string): string {
		try {
			return decodeURIComponent(value);
		} catch {
			return value;
		}
	}

	function decodeSecretPayload(value: string): string {
		try {
			return decodeURIComponent(value);
		} catch {
			return '';
		}
	}

	const SecretBlock = TiptapNode.create({
		name: 'secretBlock',
		group: 'block',
		atom: true,
		selectable: true,
		addAttributes() {
			return { payload: { default: '' } };
		},
		parseHTML() {
			return [{
				tag: 'div[data-secret-block]',
				getAttrs: (el: HTMLElement) => ({
					payload: decodeSecretPayload(el.getAttribute('data-secret-block') || ''),
				}),
			}];
		},
		renderHTML({ HTMLAttributes }) {
			const payload = HTMLAttributes.payload || '';
			return ['div', { 'data-secret-block': encodeURIComponent(payload), class: 'secret-block' }];
		},
		addNodeView() {
			return ({ node }) => {
				let payload = node.attrs.payload || '';
				const dom = document.createElement('div');
				dom.className = 'secret-block';
				dom.contentEditable = 'false';
				renderSecretNode(dom, payload);
				return {
					dom,
					update(updatedNode) {
						if (updatedNode.type.name !== 'secretBlock') return false;
						payload = updatedNode.attrs.payload || '';
						renderSecretNode(dom, payload);
						return true;
					},
					stopEvent: () => true,
					ignoreMutation: () => true,
				};
			};
		},
	});

	const MathBlock = TiptapNode.create({
		name: 'mathBlock',
		group: 'block',
		atom: true,
		addAttributes() {
			return { tex: { default: '' } };
		},
		parseHTML() {
			return [{
				tag: 'div[data-math-block]',
				getAttrs: (el: HTMLElement) => ({ tex: decodeNoteAttribute(el.getAttribute('data-math-block') || '') }),
			}];
		},
		renderHTML({ HTMLAttributes }) {
			return { dom: mathHTML(HTMLAttributes.tex || '', true) };
		},
		addNodeView() {
			return ({ node, getPos }) => {
				const dom = document.createElement('div');
				dom.classList.add('math-block');
				dom.contentEditable = 'false';
				dom.setAttribute('data-math-block', encodeURIComponent(node.attrs.tex));
				const render = () => { dom.innerHTML = renderKatex(node.attrs.tex, true); };
				if (isLargeDoc) { dom.textContent = node.attrs.tex; observeMath(dom, render); } else { render(); }
				dom.addEventListener('dblclick', (e) => {
					e.preventDefault();
					e.stopPropagation();
					const pos = typeof getPos === 'function' ? getPos() : null;
					if (pos !== null && pos !== undefined) openMathEdit(pos, 'block', node.attrs.tex);
				});
				return { dom, destroy() { mathObserver?.unobserve(dom); mathPending.delete(dom); } };
			};
		},
	});

	const MathInline = TiptapNode.create({
		name: 'mathInline',
		group: 'inline',
		inline: true,
		atom: true,
		addAttributes() {
			return { tex: { default: '' } };
		},
		parseHTML() {
			return [{
				tag: 'span[data-math-inline]',
				getAttrs: (el: HTMLElement) => ({ tex: decodeNoteAttribute(el.getAttribute('data-math-inline') || '') }),
			}];
		},
		renderHTML({ HTMLAttributes }) {
			return { dom: mathHTML(HTMLAttributes.tex || '', false) };
		},
		addNodeView() {
			return ({ node, getPos }) => {
				const dom = document.createElement('span');
				dom.classList.add('math-inline');
				dom.contentEditable = 'false';
				dom.setAttribute('data-math-inline', encodeURIComponent(node.attrs.tex));
				const render = () => { dom.innerHTML = renderKatex(node.attrs.tex, false); };
				if (isLargeDoc) { dom.textContent = node.attrs.tex; observeMath(dom, render); } else { render(); }
				dom.addEventListener('dblclick', (e) => {
					e.preventDefault();
					e.stopPropagation();
					const pos = typeof getPos === 'function' ? getPos() : null;
					if (pos !== null && pos !== undefined) openMathEdit(pos, 'inline', node.attrs.tex);
				});
				return { dom, destroy() { mathObserver?.unobserve(dom); mathPending.delete(dom); } };
			};
		},
	});

	const Callout = TiptapNode.create({
		name: 'callout',
		group: 'block',
		content: 'block+',
		defining: true,
		addAttributes() {
			return {
				type: {
					default: 'note',
					parseHTML: (el: HTMLElement) => (el.getAttribute('data-callout') || 'note').toLowerCase(),
					renderHTML: (a: Record<string, any>) => ({ 'data-callout': a.type }),
				},
				title: {
					default: '',
					parseHTML: (el: HTMLElement) => el.getAttribute('data-callout-title') || '',
					renderHTML: (a: Record<string, any>) => (a.title ? { 'data-callout-title': a.title } : {}),
				},
				foldable: {
					default: false,
					parseHTML: (el: HTMLElement) => el.getAttribute('data-callout-foldable') === 'true',
					renderHTML: (a: Record<string, any>) => ({ 'data-callout-foldable': a.foldable ? 'true' : 'false' }),
				},
				folded: {
					default: false,
					parseHTML: (el: HTMLElement) => el.getAttribute('data-callout-folded') === 'true',
					renderHTML: (a: Record<string, any>) => ({ 'data-callout-folded': a.folded ? 'true' : 'false' }),
				},
			};
		},
		parseHTML() {
			return [{ tag: 'div[data-callout]' }];
		},
		renderHTML({ node, HTMLAttributes }) {
			return ['div', mergeAttributes(HTMLAttributes, { class: 'callout', 'data-callout-group': calloutGroup(node.attrs.type) }), 0];
		},
		addNodeView() {
			return ({ node, getPos, editor }) => {
				const dom = document.createElement('div');
				const apply = (n: any) => {
					dom.className = 'callout';
					dom.classList.toggle('is-foldable', !!n.attrs.foldable);
					dom.classList.toggle('is-folded', !!n.attrs.folded && !dom.hasAttribute('data-revealed'));
					dom.setAttribute('data-callout', n.attrs.type || 'note');
					dom.setAttribute('data-callout-group', calloutGroup(n.attrs.type));
				};
				apply(node);

				const header = document.createElement('div');
				header.className = 'callout-header';
				header.contentEditable = 'false';

				const iconBtn = document.createElement('button');
				iconBtn.type = 'button';
				iconBtn.className = 'callout-icon';
				iconBtn.title = 'Change type';
				iconBtn.innerHTML = calloutIcon(node.attrs.type);

				const titleInput = document.createElement('input');
				titleInput.className = 'callout-title';
				titleInput.value = node.attrs.title || '';
				titleInput.placeholder = calloutLabel(node.attrs.type);
				titleInput.spellcheck = false;
				titleInput.readOnly = !editor.isEditable;

				const foldBtn = document.createElement('button');
				foldBtn.type = 'button';
				foldBtn.className = 'callout-fold';
				foldBtn.title = 'Fold / unfold';
				foldBtn.innerHTML = '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9"/></svg>';

				const content = document.createElement('div');
				content.className = 'callout-content';

				header.append(iconBtn, titleInput, foldBtn);
				dom.append(header, content);

				const updateAttr = (attrs: Record<string, any>) => {
					if (typeof getPos !== 'function') return;
					const pos = getPos();
					if (pos == null) return;
					const cur = editor.state.doc.nodeAt(pos);
					if (!cur) return;
					editor.view.dispatch(editor.state.tr.setNodeMarkup(pos, undefined, { ...cur.attrs, ...attrs }));
				};

				let titleDirty = false;
				const commitTitle = () => { if (titleDirty) { updateAttr({ title: titleInput.value }); titleDirty = false; } };
				titleInput.addEventListener('input', () => { titleDirty = true; });
				titleInput.addEventListener('change', commitTitle);
				titleInput.addEventListener('blur', commitTitle);
				titleInput.addEventListener('keydown', (e) => {
					if (e.key === 'Enter') { e.preventDefault(); commitTitle(); titleInput.blur(); editor.commands.focus(); }
				});

				foldBtn.addEventListener('mousedown', (e) => e.preventDefault());
				foldBtn.addEventListener('click', (e) => {
					e.preventDefault();
					// A callout revealed from the Outline is open on screen but still folded in the note.
					const revealed = dom.hasAttribute('data-revealed');
					dom.removeAttribute('data-revealed');
					const nowFolded = revealed || !dom.classList.contains('is-folded');
					if (editor.isEditable) {
						updateAttr({ folded: nowFolded, foldable: true });
						if (revealed) dom.classList.add('is-folded');
					} else {
						dom.classList.toggle('is-folded', nowFolded);
						dom.classList.add('is-foldable');
					}
				});

				iconBtn.addEventListener('mousedown', (e) => e.preventDefault());
				iconBtn.addEventListener('click', (e) => {
					e.preventDefault();
					if (!editor.isEditable) return;
					openCalloutTypeMenu(iconBtn, (newType) => updateAttr({ type: newType }));
				});

				return {
					dom,
					contentDOM: content,
					update(updated: any) {
						if (updated.type.name !== 'callout') return false;
						apply(updated);
						iconBtn.innerHTML = calloutIcon(updated.attrs.type);
						titleInput.placeholder = calloutLabel(updated.attrs.type);
						titleInput.readOnly = !editor.isEditable;
						if (document.activeElement !== titleInput) titleInput.value = updated.attrs.title || '';
						return true;
					},
					ignoreMutation(mutation: any) {
						if (mutation.type === 'selection') return false;
						return !content.contains(mutation.target as Node);
					},
					stopEvent(event: any) {
						return header.contains(event.target as Node);
					},
				};
			};
		},
	});

	/**
	 * The body of a collapsible section is isolating, so Backspace at its start did nothing and the
	 * caret could never leave it: a section could not be removed from inside, least of all when
	 * nothing sits above it. Backspace there now steps back into the summary, where the stock
	 * handler deletes text and then unwraps the section; a section with nothing in it is removed.
	 */
	const DetailsWithBodyBackspace = Details.extend({
		addKeyboardShortcuts() {
			const parent = this.parent?.() ?? {};
			return {
				...parent,
				Backspace: (props) => {
					const { state, view } = this.editor;
					const { empty, $anchor: anchor } = state.selection;
					const depth = anchor.depth - 1;
					if (empty && anchor.parentOffset === 0 && depth >= 1 && anchor.node(depth).type.name === 'detailsContent' && anchor.index(depth) === 0) {
						const content = anchor.node(depth);
						const details = anchor.node(depth - 1);
						const detailsPos = anchor.before(depth - 1);
						const summary = details.firstChild;
						if (summary?.type.name === 'detailsSummary') {
							const bodyEmpty = content.childCount === 1 && content.firstChild!.isTextblock && content.firstChild!.content.size === 0;
							if (summary.content.size === 0 && bodyEmpty) {
								const tr = state.tr.replaceWith(detailsPos, detailsPos + details.nodeSize, state.schema.nodes.paragraph.create());
								view.dispatch(tr.setSelection(TextSelection.create(tr.doc, detailsPos + 1)).scrollIntoView());
							} else {
								view.dispatch(state.tr.setSelection(TextSelection.create(state.doc, detailsPos + summary.nodeSize)).scrollIntoView());
							}
							return true;
						}
					}
					return parent.Backspace?.(props) ?? false;
				},
			};
		},
	});

	const HeadingShortcuts = Extension.create({
		name: 'headingShortcuts',
		addKeyboardShortcuts() {
			const toggle = (level: 1 | 2 | 3 | 4 | 5 | 6) => () =>
				this.editor.chain().focus().toggleHeading({ level }).run();
			return {
				'Mod-1': toggle(1),
				'Mod-2': toggle(2),
				'Mod-3': toggle(3),
				'Mod-4': toggle(4),
				'Mod-5': toggle(5),
				'Mod-6': toggle(6),
				'Mod-0': () => this.editor.chain().focus().setParagraph().run(),
			};
		},
	});

	const StrikeShortcut = Extension.create({
		name: 'strikeShortcut',
		addKeyboardShortcuts() {
			return {
				'Mod-Shift-x': () => this.editor.commands.toggleStrike(),
			};
		},
	});

	const CalloutTyping = Extension.create({
		name: 'calloutTyping',
		addProseMirrorPlugins() {
			return [
				new Plugin({
					key: new PluginKey('calloutTyping'),
					props: {
						handleDOMEvents: {
							keydown(view, event) {
								if (event.key !== 'Enter' || event.shiftKey || event.altKey || event.ctrlKey || event.metaKey) return false;
								const { state } = view;
								const { selection } = state;
								if (!selection.empty) return false;
								const head = selection.$head;
								if (head.depth !== 1 || head.parent.type.name !== 'paragraph') return false;
								if (head.parentOffset !== head.parent.content.size) return false;
								const m = head.parent.textContent.match(/^>\s*\[!([\w-]+)\]([-+]?)[ \t]*(.*)$/);
								if (!m) return false;
								event.preventDefault();
								const type = m[1].toLowerCase();
								const foldable = m[2] === '+' || m[2] === '-';
								const folded = m[2] === '-';
								const title = m[3];
								const start = head.before(1);
								const end = start + head.parent.nodeSize;
								const callout = state.schema.nodes.callout.create(
									{ type, title, foldable, folded },
									[state.schema.nodes.paragraph.create()],
								);
								const tr = state.tr.replaceWith(start, end, callout);
								tr.setSelection(TextSelection.near(tr.doc.resolve(start + 2)));
								tr.scrollIntoView();
								view.dispatch(tr);
								return true;
							},
						},
						handlePaste(view, event) {
							const text = event.clipboardData?.getData('text/plain');
							if (!text || !/^>\s*\[!/m.test(text)) return false;
							const lines = text.split('\n');
							const idx = lines.findIndex((l) => /^>\s*\[!/.test(l));
							if (idx === -1) return false;
							const m = lines[idx].match(/^>\s*\[!([\w-]+)\]([-+]?)[ \t]*(.*)$/);
							if (!m) return false;
							const type = m[1].toLowerCase();
							const foldable = m[2] === '+' || m[2] === '-';
							const folded = m[2] === '-';
							const title = m[3];
							const bodyLines = lines.slice(idx + 1).map((l) => l.replace(/^>\s?/, ''));
							while (bodyLines.length > 0 && bodyLines[bodyLines.length - 1].trim() === '') bodyLines.pop();
							const { state } = view;
							const schema = state.schema;
							const bodyContent = bodyLines.length > 0
								? bodyLines.map((l) => schema.nodes.paragraph.create(null, l ? [schema.text(l)] : []))
								: [schema.nodes.paragraph.create()];
							const callout = schema.nodes.callout.create({ type, title, foldable, folded }, bodyContent);
							const head = state.selection.$head;
							const tr = state.tr;
							let insertPos: number;
							if (head.depth >= 1 && head.parent.type.name === 'paragraph') {
								insertPos = head.before(head.depth);
								tr.replaceWith(insertPos, insertPos + head.parent.nodeSize, callout);
							} else {
								insertPos = state.selection.to;
								tr.insert(insertPos, callout);
							}
							tr.setSelection(TextSelection.near(tr.doc.resolve(insertPos + 2)));
							tr.scrollIntoView();
							view.dispatch(tr);
							return true;
						},
					},
				}),
			];
		},
	});

	const CtrlEndScrollPastEnd = Extension.create({
		name: 'ctrlEndScrollPastEnd',
		addProseMirrorPlugins() {
			return [
				new Plugin({
					key: new PluginKey('ctrlEndScrollPastEnd'),
					props: {
						handleDOMEvents: {
							keydown(view, event) {
								if (!event.ctrlKey || event.metaKey || event.altKey || event.shiftKey || event.key !== 'End') return false;
								event.preventDefault();
								const tr = view.state.tr.setSelection(TextSelection.atEnd(view.state.doc));
								view.dispatch(tr);
								(view.dom as HTMLElement).focus({ preventScroll: true });
								scrollEditorBodyToBottom(view.dom as HTMLElement);
								return true;
							},
						},
					},
				}),
			];
		},
	});

	const PageBreak = TiptapNode.create({
		name: 'pageBreak',
		group: 'block',
		atom: true,
		parseHTML() {
			return [
				{ tag: 'div[data-page-break]' },
				{
					tag: 'div',
					getAttrs: (el: HTMLElement) => {
						const style = el.getAttribute('style') || '';
						return style.includes('page-break-after') ? {} : false;
					},
				},
			];
		},
		renderHTML() {
			return ['div', { 'data-page-break': 'true', style: 'page-break-after: always; break-after: page;', class: 'page-break' }];
		},
	});

	// Remap decorations while typing; full rebuild 300ms after it settles, so large notes don't rescan the whole doc per keystroke.
	function lazyDecorationPlugin(key: PluginKey, build: (doc: any) => DecorationSet) {
		return new Plugin({
			key,
			state: {
				init: (_config: any, state: any) => build(state.doc),
				apply: (tr: any, old: DecorationSet, _oldState: any, newState: any) => {
					if (tr.getMeta(key) === 'rebuildDecos') return build(newState.doc);
					if (!tr.docChanged) return old;
					return old.map(tr.mapping, tr.doc);
				},
			},
			props: {
				decorations(state: any) { return key.getState(state); },
			},
			view() {
				let timer: ReturnType<typeof setTimeout> | null = null;
				return {
					update(view: any, prev: any) {
						if (view.state.doc === prev.doc) return;
						if (timer) clearTimeout(timer);
						timer = setTimeout(() => {
							timer = null;
							if (!view.isDestroyed) view.dispatch(view.state.tr.setMeta(key, 'rebuildDecos'));
						}, 300);
					},
					destroy() { if (timer) clearTimeout(timer); },
				};
			},
		});
	}

	const MermaidRenderer = Extension.create({
		name: 'mermaidRendererOptIn',
		addProseMirrorPlugins() {
			console.info('[HelixNotes] Mermaid renderer (opt-in) initialised');

			let mermaidPromise: Promise<any> | null = null;
			const svgCache = new Map<string, string>();
			let renderCounter = 0;

			function loadMermaid(): Promise<any> {
				if (!mermaidPromise) {
					mermaidPromise = import('mermaid')
						.then((m) => {
							const lib = m.default;
							const isDark = document.documentElement.classList.contains('dark');
							lib.initialize({
								startOnLoad: false,
								theme: isDark ? 'dark' : 'default',
								securityLevel: 'strict',
								fontFamily: 'inherit',
							});
							return lib;
						})
						.catch((e) => { console.error('[Mermaid] load failed', e); return null; });
				}
				return mermaidPromise;
			}

			function showError(container: HTMLElement, msg: string) {
				container.innerHTML = '';
				container.classList.remove('mermaid-render-loading');
				container.classList.add('mermaid-render-error');
				const text = document.createElement('div');
				text.textContent = msg;
				container.appendChild(text);
				addRetryButton(container, container.getAttribute('data-mermaid-source') || '');
			}

			function addRetryButton(container: HTMLElement, source: string) {
				const btn = document.createElement('button');
				btn.type = 'button';
				btn.className = 'mermaid-render-btn mermaid-render-btn-small';
				btn.textContent = '↻ Retry';
				btn.onclick = (e) => {
					e.preventDefault();
					e.stopPropagation();
					renderInto(container, source);
				};
				container.appendChild(btn);
			}

		async function svgToPngBlob(svgEl: SVGElement, scale = 2): Promise<Blob> {
			const clone = svgEl.cloneNode(true) as SVGElement;
			if (!clone.getAttribute('xmlns')) clone.setAttribute('xmlns', 'http://www.w3.org/2000/svg');
			if (!clone.getAttribute('xmlns:xlink')) clone.setAttribute('xmlns:xlink', 'http://www.w3.org/1999/xlink');
			// The labels inherit their font from the editor; a standalone image has nothing to inherit from.
			const shown = getComputedStyle(svgEl);
			clone.style.fontFamily = shown.fontFamily;
			clone.style.lineHeight = shown.lineHeight;

			// Sanitize: remove external resource references that taint canvas on WebKitGTK ("The operation is insecure")
			clone.querySelectorAll('image').forEach((img) => {
				const href = img.getAttribute('href') || img.getAttribute('xlink:href') || '';
				if (href.startsWith('http') || href.startsWith('//') || href.startsWith('data:')) {
					img.remove();
				}
			});

			const svgString = new XMLSerializer().serializeToString(clone);
			// Use data URL instead of blob URL — blob: URLs taint canvas on some WebKitGTK versions
			const dataUrl = 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(svgString);

			const img = new window.Image();
			await new Promise<void>((resolve, reject) => {
				img.onload = () => resolve();
				img.onerror = () => reject(new Error('SVG image load failed'));
				img.src = dataUrl;
			});
			const bbox = svgEl.getBoundingClientRect();
			const width = Math.max(Math.round(bbox.width || 800), 100);
			const height = Math.max(Math.round(bbox.height || 600), 100);
			const canvas = document.createElement('canvas');
			canvas.width = width * scale;
			canvas.height = height * scale;
			const ctx = canvas.getContext('2d');
			if (!ctx) throw new Error('Canvas not supported');
			ctx.scale(scale, scale);
			ctx.fillStyle = document.documentElement.classList.contains('dark') ? '#1e1e1e' : '#ffffff';
			ctx.fillRect(0, 0, width, height);
			ctx.drawImage(img, 0, 0, width, height);
			return await new Promise<Blob>((resolve, reject) => {
				canvas.toBlob((b) => b ? resolve(b) : reject(new Error('toBlob failed')), 'image/png');
			});
		}

			function flashToast(container: HTMLElement, msg: string) {
				const existing = container.querySelector('.mermaid-render-toast');
				if (existing) existing.remove();
				const toast = document.createElement('div');
				toast.className = 'mermaid-render-toast';
				toast.textContent = msg;
				container.appendChild(toast);
				setTimeout(() => { if (toast.parentElement) toast.remove(); }, 1500);
			}

		async function copyDiagram(container: HTMLElement) {
			const svgEl = container.querySelector('svg') as SVGElement | null;
			if (!svgEl) return;
			try {
				const blob = await svgToPngBlob(svgEl);
				const buf = new Uint8Array(await blob.arrayBuffer());
				await copyPngToClipboard(buf);
				flashToast(container, 'Copied');
			} catch (e: any) {
				console.error('[Mermaid] copy failed', e);
				// SecurityError fallback: canvas tainted by WebKitGTK — copy SVG source as text instead
				if (e instanceof DOMException && (e.name === 'SecurityError' || e.code === 18)) {
					try {
						const clone = svgEl.cloneNode(true) as SVGElement;
						if (!clone.getAttribute('xmlns')) clone.setAttribute('xmlns', 'http://www.w3.org/2000/svg');
						const svgString = new XMLSerializer().serializeToString(clone);
						await navigator.clipboard.writeText(svgString);
						flashToast(container, 'Copied SVG source (PNG blocked by browser security)');
					} catch (e2: any) {
						flashToast(container, 'Copy failed: ' + (e2?.message || String(e2)));
					}
				} else {
					flashToast(container, 'Copy failed: ' + (e?.message || String(e)));
				}
			}
		}

		async function saveDiagram(container: HTMLElement) {
			const svgEl = container.querySelector('svg') as SVGElement | null;
			if (!svgEl) return;
			try {
				const dest = await saveDialog({
					defaultPath: 'diagram.png',
					filters: [
						{ name: 'PNG Image', extensions: ['png'] },
						{ name: 'SVG Image', extensions: ['svg'] },
					],
				});
				if (!dest) return;
				const lower = dest.toLowerCase();
				if (lower.endsWith('.svg')) {
					const clone = svgEl.cloneNode(true) as SVGElement;
					if (!clone.getAttribute('xmlns')) clone.setAttribute('xmlns', 'http://www.w3.org/2000/svg');
					const svgString = new XMLSerializer().serializeToString(clone);
					await writeBytesTo(dest, new TextEncoder().encode(svgString));
				} else {
					const blob = await svgToPngBlob(svgEl);
					const buf = new Uint8Array(await blob.arrayBuffer());
					await writeBytesTo(dest, buf);
				}
				flashToast(container, 'Saved');
			} catch (e: any) {
				console.error('[Mermaid] save failed', e);
				// SecurityError fallback: canvas tainted by WebKitGTK — tell user to use SVG format
				if (e instanceof DOMException && (e.name === 'SecurityError' || e.code === 18)) {
					flashToast(container, 'PNG blocked by browser security — try SVG format');
				} else {
					flashToast(container, 'Save failed: ' + (e?.message || String(e)));
				}
			}
		}

			function addToolbar(container: HTMLElement, source: string) {
				const toolbar = document.createElement('div');
				toolbar.className = 'mermaid-render-toolbar';

				if (!isAndroid) {
					const copyBtn = document.createElement('button');
					copyBtn.type = 'button';
					copyBtn.className = 'mermaid-render-action';
					copyBtn.title = 'Copy as PNG';
					copyBtn.textContent = 'Copy';
					copyBtn.onclick = (e) => { e.preventDefault(); e.stopPropagation(); copyDiagram(container); };
					toolbar.appendChild(copyBtn);
				}

				const saveBtn = document.createElement('button');
				saveBtn.type = 'button';
				saveBtn.className = 'mermaid-render-action';
				saveBtn.title = 'Save as PNG or SVG';
				saveBtn.textContent = 'Save';
				saveBtn.onclick = (e) => { e.preventDefault(); e.stopPropagation(); saveDiagram(container); };

				const reRenderBtn = document.createElement('button');
				reRenderBtn.type = 'button';
				reRenderBtn.className = 'mermaid-render-action';
				reRenderBtn.title = 'Re-render diagram';
				reRenderBtn.textContent = '↻';
				reRenderBtn.onclick = (e) => {
					e.preventDefault();
					e.stopPropagation();
					svgCache.delete(source);
					renderInto(container, source);
				};

				toolbar.appendChild(saveBtn);
				toolbar.appendChild(reRenderBtn);
				container.appendChild(toolbar);
			}

			async function renderInto(container: HTMLElement, source: string) {
				container.innerHTML = '';
				container.classList.remove('mermaid-render-error', 'mermaid-render-idle');
				container.classList.add('mermaid-render-loading');

				const cached = svgCache.get(source);
				if (cached) {
					container.classList.remove('mermaid-render-loading');
					container.innerHTML = cached;
					addToolbar(container, source);
					return;
				}

				const mermaid = await loadMermaid();
				if (!mermaid) {
					showError(container, 'Mermaid library failed to load.');
					return;
				}
				try {
					const parseOk = await mermaid.parse(source, { suppressErrors: true });
					if (!parseOk) {
						showError(container, 'Invalid mermaid syntax.');
						return;
					}
					const id = `mermaid-${++renderCounter}`;
					// Measure labels where the diagram is shown: measured under <body>, they get a
					// different line height than the editor's and the last line of a box is cut off.
					const { svg } = await mermaid.render(id, source, container.isConnected ? container : undefined);
					svgCache.set(source, svg);
					if (container.isConnected) {
						container.classList.remove('mermaid-render-loading');
						container.innerHTML = svg;
						addToolbar(container, source);
					}
				} catch (e: any) {
					showError(container, 'Render failed: ' + (e?.message || String(e)));
				}
			}

			function makeIdleButton(source: string): HTMLElement {
				const container = document.createElement('div');
				container.className = 'mermaid-render mermaid-render-idle';
				container.contentEditable = 'false';
				container.setAttribute('data-mermaid-source', source);

				const cached = svgCache.get(source);
				if (cached) {
					container.classList.remove('mermaid-render-idle');
					container.innerHTML = cached;
					addToolbar(container, source);
					return container;
				}

				const btn = document.createElement('button');
				btn.type = 'button';
				btn.className = 'mermaid-render-btn';
				btn.textContent = '▶  Render diagram';
				btn.onclick = (e) => {
					e.preventDefault();
					e.stopPropagation();
					renderInto(container, source);
				};
				container.appendChild(btn);
				return container;
			}

			function buildDecorations(doc: any): DecorationSet {
				const decos: any[] = [];
				doc.descendants((node: any, pos: number) => {
					if (node.type.name === 'codeBlock' && node.attrs.language === 'mermaid') {
						const source = node.textContent;
						if (!source.trim()) return;
						decos.push(
							Decoration.widget(
								pos + node.nodeSize,
								() => makeIdleButton(source),
								{ side: 1, key: 'mermaid:' + source.length + ':' + (svgCache.has(source) ? 'r' : 'i') },
							),
						);
					}
				});
				return DecorationSet.create(doc, decos);
			}

			const pluginKey = new PluginKey('mermaidRendererOptIn');
			return [lazyDecorationPlugin(pluginKey, buildDecorations)];
		},
	});

	const CopyButtonExtension = Extension.create({
		name: 'codeBlockCopyButton',
		addProseMirrorPlugins() {
			return [lazyDecorationPlugin(new PluginKey('codeBlockCopyButton'), buildCodeCopyDecorations)];
		},
	});

	const FocusedPlaceholder = Extension.create({
		name: 'placeholder',
		addProseMirrorPlugins() {
			return [new Plugin({
				key: new PluginKey('placeholder'),
				props: { decorations: (state) => this.editor.isEditable ? focusedPlaceholderDecorations(state) : null },
			})];
		},
	});

	let codeLangDropdown = $state<{ pos: number; x: number; y: number; current: string } | null>(null);
	let codeLangSearch = $state('');
	let codeLangInput = $state<HTMLInputElement | null>(null);

	let codeLangFiltered = $derived.by(() => {
		if (!codeLangSearch) return codeLanguages;
		const q = codeLangSearch.toLowerCase();
		return codeLanguages.filter(l => l.includes(q));
	});

	function openCodeLangDropdown(pos: number, current: string, triggerEl: HTMLElement) {
		const rect = triggerEl.getBoundingClientRect();
		codeLangSearch = '';
		codeLangDropdown = { pos, x: rect.right, y: rect.bottom + 4, current };
		const origin = captureOrigin();
		tick().then(() => { if (codeLangDropdown && isOriginCurrent(origin)) codeLangInput?.focus(); });
	}

	function selectCodeLang(lang: string) {
		if (!editor || !codeLangDropdown) return;
		const { pos } = codeLangDropdown;
		// Find the codeBlock node at this position
		const resolved = editor.state.doc.resolve(pos);
		const node = resolved.parent;
		if (node.type.name === 'codeBlock') {
			editor.chain().focus().updateAttributes('codeBlock', { language: lang || null }).run();
		}
		codeLangDropdown = null;
	}

	function closeCodeLangDropdown() {
		codeLangDropdown = null;
	}

	// ── In-note search extension ──
	const NoteSearchExtension = Extension.create({
		name: 'noteSearch',
		addProseMirrorPlugins() {
			return [
				new Plugin({
					key: noteSearchPluginKey,
					state: {
						init() { return DecorationSet.empty; },
						apply(tr, old) {
							const meta = tr.getMeta(noteSearchPluginKey);
							if (meta !== undefined) return meta;
							return tr.docChanged ? DecorationSet.empty : old;
						},
					},
					props: {
						decorations(state) {
							return this.getState(state);
						},
					},
				}),
			];
		},
	});

	const TargetRevealExtension = Extension.create({
		name: 'targetReveal',
		addProseMirrorPlugins() {
			return [
				new Plugin({
					key: targetRevealPluginKey,
					state: {
						init() { return DecorationSet.empty; },
						apply(tr, old) {
							const meta = tr.getMeta(targetRevealPluginKey);
							if (meta !== undefined) return meta;
							return old.map(tr.mapping, tr.doc);
						},
					},
					props: {
						decorations(state) { return this.getState(state); },
					},
				}),
			];
		},
	});

	// Color swatch decorations: render a small filled square before every hex/rgb/hsl color
	// literal (in normal text AND code blocks), VSCode-style. These are view-only widget
	// decorations, so they never touch the document/markdown - the note stores only the plain
	// color text and the swatch is re-derived on load.
	const colorSwatchPluginKey = new PluginKey('colorSwatch');
	const COLOR_LITERAL_RE = /#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{3,4})\b|(?:rgb|rgba|hsl|hsla)\([^)\n]{1,64}\)/g;

	function makeColorSwatch(color: string): HTMLElement {
		const span = document.createElement('span');
		span.className = 'color-swatch';
		span.contentEditable = 'false';
		span.style.backgroundColor = color;
		return span;
	}

	function buildColorSwatchDecorations(doc: any): DecorationSet {
		const decos: any[] = [];
		doc.descendants((node: any, pos: number) => {
			if (!node.isText || !node.text) return;
			const text: string = node.text;
			COLOR_LITERAL_RE.lastIndex = 0;
			let m: RegExpExecArray | null;
			while ((m = COLOR_LITERAL_RE.exec(text)) !== null) {
				const color = m[0];
				// Validate with the browser's own CSS parser so junk (rgb(foo), #12345) gets no swatch.
				if (!CSS.supports('color', color)) continue;
				const at = pos + m.index;
				decos.push(Decoration.widget(at, () => makeColorSwatch(color), { side: -1, key: 'cs:' + color + '@' + at }));
			}
		});
		return DecorationSet.create(doc, decos);
	}

	const ColorSwatch = Extension.create({
		name: 'colorSwatch',
		addProseMirrorPlugins() {
			return [lazyDecorationPlugin(colorSwatchPluginKey, buildColorSwatchDecorations)];
		},
	});

	// Dim the inline task metadata tokens (!high / !medium / !med / !low and due:YYYY-MM-DD) inside
	// task items so they recede visually without disappearing. View-only inline decorations - the
	// note text is untouched. Token shapes mirror the parser in commands.rs.
	const taskMetaPluginKey = new PluginKey('taskMetaDim');

	const TaskMetaDim = Extension.create({
		name: 'taskMetaDim',
		addProseMirrorPlugins() {
			return [lazyDecorationPlugin(taskMetaPluginKey, buildTaskMetaDecorations)];
		},
	});

	const MoveLineShortcuts = Extension.create({
		name: 'moveLineShortcuts',
		addProseMirrorPlugins() {
			return [
				new Plugin({
					key: new PluginKey('moveLineShortcuts'),
					props: {
						handleDOMEvents: {
							keydown(view, event) {
								if (!event.altKey || (event.key !== 'ArrowUp' && event.key !== 'ArrowDown')) return false;
								event.preventDefault();
								const { state, dispatch } = view;
								const resolvedPos = state.selection.$from;
								if (event.shiftKey) {
									for (let depth = resolvedPos.depth; depth > 0; depth--) {
										const itemNode = resolvedPos.node(depth);
										if (itemNode.type.name !== 'listItem' && itemNode.type.name !== 'taskItem') continue;
										const parentListDepth = depth - 1;
										const itemIndex = resolvedPos.index(parentListDepth);
										const itemPos = resolvedPos.before(depth);
										const itemSlice = state.doc.slice(itemPos, itemPos + itemNode.nodeSize);
										const cursorOffset = resolvedPos.pos - itemPos;
										const tr = state.tr;

										if (event.key === 'ArrowUp') {
											if (itemIndex <= 0) return true;
											const prevPos = resolvedPos.posAtIndex(itemIndex - 1, parentListDepth);
											tr.delete(itemPos, itemPos + itemNode.nodeSize);
											const insertAt = tr.mapping.map(prevPos);
											tr.insert(insertAt, itemSlice.content);
											const newCursorPos = Math.min(insertAt + cursorOffset, tr.doc.content.size);
											tr.setSelection(Selection.near(tr.doc.resolve(newCursorPos)));
											dispatch(tr.scrollIntoView());
											return true;
										}

										const parentList = resolvedPos.node(parentListDepth);
										if (itemIndex >= parentList.childCount - 1) return true;
										const nextPos = resolvedPos.posAtIndex(itemIndex + 1, parentListDepth);
										const nextNode = state.doc.nodeAt(nextPos);
										if (!nextNode) return true;
										const nextSlice = state.doc.slice(nextPos, nextPos + nextNode.nodeSize);
										tr.delete(nextPos, nextPos + nextNode.nodeSize);
										const insertAt = tr.mapping.map(itemPos);
										tr.insert(insertAt, nextSlice.content);
										const newCursorPos = Math.min(tr.mapping.map(itemPos) + cursorOffset, tr.doc.content.size);
										tr.setSelection(Selection.near(tr.doc.resolve(newCursorPos)));
										dispatch(tr.scrollIntoView());
										return true;
									}
									return true;
								}
								// Find the top-level block index
								const depth = 1; // top-level blocks in doc
								if (resolvedPos.depth < depth) return true;
								const parentPos = resolvedPos.before(depth);
								const parentNode = state.doc.nodeAt(parentPos);
								if (!parentNode) return true;
								const parentIndex = resolvedPos.index(0);
								if (event.key === 'ArrowUp') {
									if (parentIndex <= 0) return true;
									const prevPos = resolvedPos.posAtIndex(parentIndex - 1, 0);
									const prevNode = state.doc.nodeAt(prevPos);
									if (!prevNode) return true;
									const tr = state.tr;
									const cursorOffset = resolvedPos.pos - parentPos;
									// Delete current block, insert it before previous block
									const curSlice = state.doc.slice(parentPos, parentPos + parentNode.nodeSize);
									tr.delete(parentPos, parentPos + parentNode.nodeSize);
									const insertAt = tr.mapping.map(prevPos);
									tr.insert(insertAt, curSlice.content);
									const newCursorPos = Math.min(insertAt + cursorOffset, tr.doc.content.size);
									tr.setSelection(Selection.near(tr.doc.resolve(newCursorPos)));
									dispatch(tr.scrollIntoView());
								} else {
									if (parentIndex >= state.doc.childCount - 1) return true;
									const nextPos = resolvedPos.posAtIndex(parentIndex + 1, 0);
									const nextNode = state.doc.nodeAt(nextPos);
									if (!nextNode) return true;
									const tr = state.tr;
									const cursorOffset = resolvedPos.pos - parentPos;
									// Delete next block, insert it before current block
									const nextSlice = state.doc.slice(nextPos, nextPos + nextNode.nodeSize);
									tr.delete(nextPos, nextPos + nextNode.nodeSize);
									const insertAt = tr.mapping.map(parentPos);
									tr.insert(insertAt, nextSlice.content);
									const newCursorPos = Math.min(tr.mapping.map(parentPos) + cursorOffset, tr.doc.content.size);
									tr.setSelection(Selection.near(tr.doc.resolve(newCursorPos)));
									dispatch(tr.scrollIntoView());
								}
								return true;
							},
						},
					},
				}),
			];
		},
	});

	// Tab inserts a tab character in plain paragraphs/headings.
	// Priority 50 < default 100, so list/task/table/codeblock extensions handle Tab first for their own nodes.
	const TabIndent = Extension.create({
		name: 'tabIndent',
		priority: 50,
		addKeyboardShortcuts() {
			return {
				Tab: () => {
					const sel = this.editor.state.selection;
					const from = sel.$from;
					const node = from.node();
					if (node.type.name !== 'paragraph' && node.type.name !== 'heading') return false;
					// Don't intercept if inside a list or task item (their extensions handle Tab first,
					// but guard here too in case of priority edge cases)
					for (let d = from.depth - 1; d > 0; d--) {
						const name = from.node(d).type.name;
						if (name === 'listItem' || name === 'taskItem') return false;
					}
					return this.editor.commands.insertContent('\t');
				},
			};
		},
	});

	const CodeBlockLanguageSelect = Extension.create({
		name: 'codeBlockLanguageSelect',
		addGlobalAttributes() {
			return [{
				types: ['codeBlock'],
				attributes: {
					language: {
						renderHTML: (attributes) => {
							return { 'data-language': attributes.language || '' };
						},
					},
				},
			}];
		},
		addProseMirrorPlugins() {
			return [
				new Plugin({
					key: new PluginKey('codeBlockLanguageSelect'),
					props: {
						handleDOMEvents: {
							click: (view, event) => {
								const target = event.target as HTMLElement;
								const pre = target.closest('pre');
								if (!pre) return false;
								// Check if click is in the top-right corner (language button area)
								const rect = pre.getBoundingClientRect();
								if (event.clientX < rect.right - 100 || event.clientY > rect.top + 30) return false;
								// Find the code block position
								const pos = view.posAtDOM(pre, 0);
								const resolved = view.state.doc.resolve(pos);
								let cbNode = resolved.parent;
								let cbPos = resolved.before(resolved.depth);
								if (cbNode.type.name !== 'codeBlock') {
									for (let d = resolved.depth; d >= 0; d--) {
										if (resolved.node(d).type.name === 'codeBlock') {
											cbNode = resolved.node(d);
											cbPos = resolved.before(d);
											break;
										}
									}
								}
								if (cbNode.type.name !== 'codeBlock') return false;
								event.preventDefault();
								event.stopPropagation();
								// Virtual trigger for dropdown positioning
								const triggerEl = document.createElement('div');
								triggerEl.getBoundingClientRect = () => ({
									top: rect.top + 5, bottom: rect.top + 25,
									left: rect.right - 100, right: rect.right - 34,
									width: 66, height: 20,
									x: rect.right - 100, y: rect.top + 5,
									toJSON() { return this; },
								});
								openCodeLangDropdown(cbPos + 1, cbNode.attrs.language || '', triggerEl as any);
								return true;
							},
						},
					},
				}),
			];
		},
	});

	function executeSlashCommand(index: number) {
		const items = slashFiltered;
		if (index < 0 || index >= items.length || !slashMenu || !editor) return;
		const cmd = items[index];
		// Table opens a sub-picker instead of closing
		if (cmd.label === 'Table') {
			slashTablePicker = true;
			slashTableHover = { rows: 0, cols: 0 };
			slashSelectedIndex = 0;
			// Delete slash text after setting flag so onTransaction doesn't close menu
			editor.chain().focus().deleteRange({ from: slashMenu.from, to: slashMenu.to }).run();
			return;
		}
		// Color opens a sub-picker too
		if (cmd.label === 'Color') {
			slashColorPicker = true;
			slashSelectedIndex = 0;
			editor.chain().focus().deleteRange({ from: slashMenu.from, to: slashMenu.to }).run();
			const origin = captureOrigin();
			tick().then(() => { if (slashColorPicker && isOriginCurrent(origin)) slashColorInputEl?.focus(); });
			return;
		}
		// Delete the slash trigger text (/ + query)
		editor.chain().focus().deleteRange({ from: slashMenu.from, to: slashMenu.to }).run();
		slashMenu = null;
		slashSelectedIndex = 0;
		// Execute after the deletion is applied
		const origin = captureOrigin();
		tick().then(() => { if (canInsert(origin) && isOriginCurrent(origin, true)) cmd.action(); });
	}

	function slashInsertTable(rows: number, cols: number) {
		if (!editor) return;
		editor.chain().focus().insertTable({ rows, cols, withHeaderRow: true }).run();
		closeSlashMenu();
	}

	function insertColor(color: string) {
		if (!editor) return;
		const c = (color || '').trim();
		if (!c || !CSS.supports('color', c)) { closeSlashMenu(); return; }
		editor.chain().focus().insertContent(c).run();
		closeSlashMenu();
	}

	// Track whether the user just typed a slash (vs cursor moving into existing text)
	let slashTypedByUser = false;

	function closeSlashMenu() {
		slashMenu = null;
		slashSelectedIndex = 0;
		slashTablePicker = false;
		slashTableHover = { rows: 0, cols: 0 };
		slashColorPicker = false;
	}

	$effect(() => {
		if (!slashMenu || slashSelectedIndex < 0) return;
		slashSelectedIndex; // track
		tick().then(() => {
			document.querySelector('.slash-menu .slash-menu-item.selected')?.scrollIntoView({ block: 'nearest' });
		});
	});

	$effect(() => {
		if (!wikiLinkMenu || wikiLinkSelectedIndex < 0) return;
		wikiLinkSelectedIndex; // track
		tick().then(() => {
			document.querySelector('.wiki-link-menu .wiki-link-item.selected')?.scrollIntoView({ block: 'nearest' });
		});
	});

	function updateSlashMenu() {
		const wasSlashTyped = slashTypedByUser;
		slashTypedByUser = false;
		if (!editor) return;
		if (slashTablePicker || slashColorPicker) return; // a sub-picker is open, don't interfere
		const { state } = editor;
		const { selection } = state;
		const resolvedFrom = selection.$from;

		// Only in empty-ish context (paragraph, heading)
		const parentNode = resolvedFrom.parent;
		if (parentNode.type.name !== 'paragraph' && parentNode.type.name !== 'heading') {
			closeSlashMenu();
			return;
		}

		const textBefore = parentNode.textContent.slice(0, resolvedFrom.parentOffset);
		// Match "/" at start of line or after whitespace
		const match = textBefore.match(/(^|\s)\/([^\s]*)$/);
		if (!match) {
			closeSlashMenu();
			return;
		}

		// Only open the menu if the user typed the slash, or the menu is already open
		// This prevents triggering when clicking/arrowing into existing paths like /usr/local/bin
		if (!slashMenu && !wasSlashTyped) {
			return;
		}

		const query = match[2];
		const slashOffset = textBefore.length - match[0].length + (match[1].length); // position of "/"
		const from = resolvedFrom.start() + slashOffset;
		const to = resolvedFrom.pos;

		// Get cursor coordinates for menu positioning
		const coords = editor.view.coordsAtPos(from);

		let x = coords.left;

		// Keep menu within viewport (account for virtual keyboard on mobile)
		if (x + 240 > window.innerWidth) x = window.innerWidth - 250;
		const vv1 = window.visualViewport;
		const visibleBottom = isMobile && vv1 ? vv1.height + (vv1.offsetTop || 0) : window.innerHeight;
		const menuHeight = 300;
		let y = coords.bottom + 4;
		if (y + menuHeight > visibleBottom) y = coords.top - menuHeight - 4;
		if (y < 4) y = 4;

		slashMenu = { x, y, query, from, to };
		slashSelectedIndex = 0;
	}

	const SlashCommands = Extension.create({
		name: 'slashCommands',
		addProseMirrorPlugins() {
			return [
				new Plugin({
					key: new PluginKey('slashCommands'),
					props: {
						handleTextInput: (_view, _from, _to, text) => {
							if (text === '/') {
								slashTypedByUser = true;
							}
							return false;
						},
						handleKeyDown: (_view, event) => {
							if (!slashMenu) return false;
							if (slashColorPicker) {
								if (event.key === 'Escape') {
									event.preventDefault();
									closeSlashMenu();
								}
								return true; // the picker's own inputs handle the rest
							}
							if (slashTablePicker) {
								if (event.key === 'Escape') {
									event.preventDefault();
									closeSlashMenu();
									return true;
								}
								if (event.key === 'Tab') {
									event.preventDefault();
									if (slashTableHover.rows > 0 && slashTableHover.cols > 0) {
										slashInsertTable(slashTableHover.rows, slashTableHover.cols);
									}
									return true;
								}
								if (event.key === 'ArrowRight') {
									event.preventDefault();
									slashTableHover = { rows: Math.max(1, slashTableHover.rows), cols: Math.min(10, (slashTableHover.cols || 0) + 1) };
									return true;
								}
								if (event.key === 'ArrowLeft') {
									event.preventDefault();
									slashTableHover = { rows: Math.max(1, slashTableHover.rows), cols: Math.max(1, slashTableHover.cols - 1) };
									return true;
								}
								if (event.key === 'ArrowDown') {
									event.preventDefault();
									slashTableHover = { rows: Math.min(8, (slashTableHover.rows || 0) + 1), cols: Math.max(1, slashTableHover.cols) };
									return true;
								}
								if (event.key === 'ArrowUp') {
									event.preventDefault();
									slashTableHover = { rows: Math.max(1, slashTableHover.rows - 1), cols: Math.max(1, slashTableHover.cols) };
									return true;
								}
								if (event.key === 'Enter' || event.key === ' ') {
									event.preventDefault();
									if (slashTableHover.rows > 0 && slashTableHover.cols > 0) {
										slashInsertTable(slashTableHover.rows, slashTableHover.cols);
									}
									return true;
								}
								return true;
							}
							if (event.key === 'ArrowDown') {
								event.preventDefault();
								slashSelectedIndex = (slashSelectedIndex + 1) % Math.max(1, slashFiltered.length);
								return true;
							}
							if (event.key === 'ArrowUp') {
								event.preventDefault();
								slashSelectedIndex = (slashSelectedIndex - 1 + slashFiltered.length) % Math.max(1, slashFiltered.length);
								return true;
							}
							if (event.key === 'Enter' || event.key === 'Tab') {
								if (slashFiltered.length > 0) {
									event.preventDefault();
									executeSlashCommand(slashSelectedIndex);
									return true;
								}
								closeSlashMenu();
								return false;
							}
							if (event.key === 'Escape') {
								event.preventDefault();
								closeSlashMenu();
								return true;
							}
							return false;
						},
					},
				}),
			];
		},
	});

	// ── Task metadata (! priority / due) ──

	const TASK_META_ITEMS = [
		{ label: 'High priority', token: '!high', kind: 'prio', icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 15s1-1 4-1 5 2 8 2 4-1 4-1V3s-1 1-4 1-5-2-8-2-4 1-4 1z"/><line x1="4" y1="22" x2="4" y2="15"/></svg>' },
		{ label: 'Medium priority', token: '!med', kind: 'prio', icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 15s1-1 4-1 5 2 8 2 4-1 4-1V3s-1 1-4 1-5-2-8-2-4 1-4 1z"/><line x1="4" y1="22" x2="4" y2="15"/></svg>' },
		{ label: 'Low priority', token: '!low', kind: 'prio', icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 15s1-1 4-1 5 2 8 2 4-1 4-1V3s-1 1-4 1-5-2-8-2-4 1-4 1z"/><line x1="4" y1="22" x2="4" y2="15"/></svg>' },
		{ label: 'Due date', token: '', kind: 'due', icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="18" rx="2"/><line x1="16" y1="2" x2="16" y2="6"/><line x1="8" y1="2" x2="8" y2="6"/><line x1="3" y1="10" x2="21" y2="10"/></svg>' },
	];
	let taskMetaMenu = $state<{ x: number; y: number; from: number; to: number; query: string } | null>(null);
	let taskMetaSelectedIndex = $state(0);
	let taskMetaTypedByUser = false;
	let taskDuePicker = $state<{ x: number; y: number } | null>(null);
	let taskDueInputEl = $state<HTMLInputElement | null>(null);

	let taskMetaFiltered = $derived.by(() => {
		const q = (taskMetaMenu?.query || '').toLowerCase();
		if (!q) return TASK_META_ITEMS;
		return TASK_META_ITEMS.filter((it) => it.label.toLowerCase().startsWith(q) || it.token.slice(1).startsWith(q));
	});

	function closeTaskMetaMenu() {
		taskMetaMenu = null;
		taskMetaSelectedIndex = 0;
	}

	function updateTaskMetaMenu() {
		const wasTyped = taskMetaTypedByUser;
		taskMetaTypedByUser = false;
		if (!editor || taskDuePicker) return;
		const rfrom = editor.state.selection.$from;
		let inTask = false;
		for (let d = rfrom.depth; d > 0; d--) {
			if (rfrom.node(d).type.name === 'taskItem') { inTask = true; break; }
		}
		if (!inTask || rfrom.parent.type.name !== 'paragraph') { closeTaskMetaMenu(); return; }
		const textBefore = rfrom.parent.textContent.slice(0, rfrom.parentOffset);
		const match = textBefore.match(/(^|\s)!([a-z]*)$/i);
		if (!match) { closeTaskMetaMenu(); return; }
		if (!taskMetaMenu && !wasTyped) return;
		const query = match[2];
		const offset = textBefore.length - match[0].length + match[1].length;
		const from = rfrom.start() + offset;
		const to = rfrom.pos;
		const coords = editor.view.coordsAtPos(from);
		let x = coords.left;
		if (x + 200 > window.innerWidth) x = window.innerWidth - 210;
		const vv2 = window.visualViewport;
		const visibleBottom = isMobile && vv2 ? vv2.height + (vv2.offsetTop || 0) : window.innerHeight;
		const menuH = 180;
		let y = coords.bottom + 4;
		if (y + menuH > visibleBottom) y = coords.top - menuH - 4;
		if (y < 4) y = 4;
		taskMetaMenu = { x, y, from, to, query };
		if (taskMetaSelectedIndex >= taskMetaFiltered.length) taskMetaSelectedIndex = 0;
	}

	function selectTaskMeta(index: number) {
		const items = taskMetaFiltered;
		if (index < 0 || index >= items.length || !taskMetaMenu || !editor) return;
		const item = items[index];
		const { from, to, x, y } = taskMetaMenu;
		taskMetaMenu = null;
		taskMetaSelectedIndex = 0;
		if (item.kind === 'due') {
			editor.chain().focus().deleteRange({ from, to }).run();
			taskDuePicker = { x, y };
			const origin = captureOrigin();
			tick().then(() => { if (taskDuePicker && isOriginCurrent(origin)) { taskDueInputEl?.focus(); taskDueInputEl?.showPicker?.(); } });
			return;
		}
		editor.chain().focus().deleteRange({ from, to }).insertContent(item.token + ' ').run();
	}

	function applyTaskDue(value: string) {
		taskDuePicker = null;
		if (!editor) return;
		if (value) editor.chain().focus().insertContent(`due:${value} `).run();
		else editor.commands.focus();
	}

	const TaskMetaMenu = Extension.create({
		name: 'taskMetaMenu',
		addProseMirrorPlugins() {
			return [
				new Plugin({
					key: new PluginKey('taskMetaMenu'),
					props: {
						handleTextInput: (_view, _from, _to, text) => {
							if (text === '!') taskMetaTypedByUser = true;
							return false;
						},
						handleKeyDown: (_view, event) => {
							if (!taskMetaMenu) return false;
							const n = Math.max(1, taskMetaFiltered.length);
							if (event.key === 'ArrowDown') { event.preventDefault(); taskMetaSelectedIndex = (taskMetaSelectedIndex + 1) % n; return true; }
							if (event.key === 'ArrowUp') { event.preventDefault(); taskMetaSelectedIndex = (taskMetaSelectedIndex - 1 + n) % n; return true; }
							if (event.key === 'Enter' || event.key === 'Tab') {
								if (taskMetaFiltered.length > 0) { event.preventDefault(); selectTaskMeta(taskMetaSelectedIndex); return true; }
								closeTaskMetaMenu();
								return false;
							}
							if (event.key === 'Escape') { event.preventDefault(); closeTaskMetaMenu(); return true; }
							return false;
						},
					},
				}),
			];
		},
	});

	// ── Wiki-links ──

	let wikiLinkMenu = $state<{ x: number; y: number; query: string; from: number } | null>(null);
	let wikiLinkSelectedIndex = $state(0);
	// Replaced, never mutated: the markdown parser keeps one lookup index per array.
	let wikiLinkTitlesCache = $state.raw<NoteTitleEntry[]>([]);
	let wikiLinkTypedByUser = false;
	// Disambiguation state: when ]] auto-close finds multiple matches
	let wikiLinkDisambigEntries = $state<NoteTitleEntry[] | null>(null);
	let wikiLinkDisambigRef = $state<string | null>(null);
	let wikiLinkDisambigDisplay = $state<string | null>(null);
	// Navigation disambiguation: when clicking a wikilink with multiple matches
	let wikiLinkNavDisambig = $state<{ entries: NoteTitleEntry[]; x: number; y: number } | null>(null);
	let wikiLinkNavDisambigIndex = $state(0);
	// Tracks the wiki-link mark under the cursor across transactions, used to detect
	// when the cursor leaves a link so we can rename the linked note if needed.
	let prevCursorWikiMark: any = null;

	let wikiLinkFiltered = $derived.by(() => {
		// When disambiguating, show only the exact matches
		if (wikiLinkDisambigEntries) return wikiLinkDisambigEntries;
		if (!wikiLinkMenu) return wikiLinkTitlesCache;
		let q = wikiLinkMenu.query.toLowerCase();
		if (!q) return wikiLinkTitlesCache;
		// Strip |alias, #heading, ^block - only use the note name part for filtering
		const pipeIdx = q.indexOf('|');
		if (pipeIdx >= 0) q = q.slice(0, pipeIdx);
		q = q.replace(/#.*$/, '').replace(/\^.*$/, '').trim();
		if (!q) return wikiLinkTitlesCache;
		// Score: 0 = exact, 1 = starts-with, 2 = word-start, 3 = contains
		const scored = wikiLinkTitlesCache
			.map(entry => {
				const t = entry.title.toLowerCase();
				let score: number;
				if (t === q) score = 0;
				else if (t.startsWith(q)) score = 1;
				else if (t.includes(' ' + q) || t.includes('-' + q)) score = 2;
				else if (t.includes(q)) score = 3;
				else score = -1;
				return { entry, score };
			})
			.filter(x => x.score >= 0)
			.sort((a, b) => a.score - b.score)
			.map(x => x.entry);
		return scored;
	});

	// Set of lowercase titles that appear more than once (for disambiguation display)
	let wikiLinkDuplicateTitles = $derived.by(() => {
		const counts = new Map<string, number>();
		for (const e of wikiLinkTitlesCache) {
			const key = e.title.toLowerCase();
			counts.set(key, (counts.get(key) ?? 0) + 1);
		}
		const dupes = new Set<string>();
		for (const [key, count] of counts) {
			if (count > 1) dupes.add(key);
		}
		return dupes;
	});

	function wikiLinkFolderPath(entry: NoteTitleEntry): string {
		if (!entry.path) return '';
		// entry.path is vault-relative (e.g. "folder/note.md"); normalize Windows backslashes.
		const path = entry.path.replace(/\\/g, '/');
		const parts = path.split('/');
		// Return parent folder(s), excluding the filename
		return parts.length > 1 ? parts.slice(0, -1).join('/') + '/' : '';
	}

	let wikiTitleRefresh: Promise<void> | null = null;
	let wikiTitleVault: string | null = null;
	let wikiTitleRefreshTimer: ReturnType<typeof setTimeout> | null = null;
	const pendingWikiTitleChanges = new Map<string, NoteTitleEntry | null>();
	let editorDisposed = false;
	let editorNavigationToken: number | null = null;

	function beginEditorNoteNavigation(): () => boolean {
		const token = beginNoteNavigation();
		editorNavigationToken = token;
		const vault = $appConfig?.active_vault;
		const path = $activeNotePath;
		const session = saves.capture();
		return () => !editorDisposed
			&& isCurrentNoteNavigation(token)
			&& vault === $appConfig?.active_vault
			&& path === $activeNotePath
			&& saves.isCurrentSession(session);
	}

	async function refreshWikiLinkTitles() {
		const vault = $appConfig?.active_vault;
		if (editorDisposed || !vault) return;
		if (vault !== wikiTitleVault) {
			wikiTitleVault = vault;
			wikiTitleRefresh = null;
			wikiLinkTitlesCache = [];
			pendingWikiTitleChanges.clear();
		}
		if (wikiTitleRefresh) return wikiTitleRefresh;
		const request = getAllNoteTitles(vault).then((titles) => {
			if (editorDisposed || vault !== $appConfig?.active_vault || wikiTitleRefresh !== request) return;
			if (pendingWikiTitleChanges.size) {
				const entries = new Map(titles.map(entry => [entry.path, entry]));
				for (const [path, entry] of pendingWikiTitleChanges) {
					if (entry) entries.set(path, entry);
					else entries.delete(path);
				}
				wikiLinkTitlesCache = [...entries.values()];
			} else {
				wikiLinkTitlesCache = titles;
			}
		}).catch((e) => {
			console.error('Failed to load note titles:', e);
		}).finally(() => {
			if (wikiTitleRefresh === request) {
				wikiTitleRefresh = null;
				pendingWikiTitleChanges.clear();
			}
		});
		wikiTitleRefresh = request;
		return wikiTitleRefresh;
	}

	function scheduleWikiTitleRefresh() {
		if (wikiTitleRefreshTimer !== null) clearTimeout(wikiTitleRefreshTimer);
		wikiTitleRefreshTimer = setTimeout(() => {
			wikiTitleRefreshTimer = null;
			// A structural event during a scan needs a fresh scan after that scan finishes.
			if (wikiTitleRefresh) void wikiTitleRefresh.then(refreshWikiLinkTitles);
			else void refreshWikiLinkTitles();
		}, 250);
	}

	function handleWikiFileChange(event: FileEvent) {
		if (!$appConfig?.enable_wiki_links || !isCurrentVaultEvent(event, $appConfig.active_vault)) return;
		const path = normalizeWikiPath(event.note?.relative_path ?? event.path).replace(/\\/g, '/');
		if (event.note) {
			const entry = { path, title: event.note.meta.title };
			const index = wikiLinkTitlesCache.findIndex(note => note.path === path);
			if (index >= 0) {
				if (wikiLinkTitlesCache[index].title !== entry.title) {
					const updated = wikiLinkTitlesCache.slice();
					updated[index] = entry;
					wikiLinkTitlesCache = updated;
				}
			} else {
				wikiLinkTitlesCache = [...wikiLinkTitlesCache, entry];
			}
			if (wikiTitleRefresh) pendingWikiTitleChanges.set(path, entry);
		} else if (event.event_type === 'remove') {
			wikiLinkTitlesCache = wikiLinkTitlesCache.filter(entry => entry.path !== path && !entry.path.startsWith(path + '/'));
			if (wikiTitleRefresh) pendingWikiTitleChanges.set(path, null);
			if (!path.toLowerCase().endsWith('.md')) scheduleWikiTitleRefresh();
		} else {
			// Directories and events without note metadata can add, rename or remove descendants.
			scheduleWikiTitleRefresh();
		}
	}

	function closeWikiLinkMenu() {
		wikiLinkMenu = null;
		wikiLinkSelectedIndex = 0;
		wikiLinkDisambigEntries = null;
		wikiLinkDisambigRef = null;
		wikiLinkDisambigDisplay = null;
	}

	function wikiLinkRelPath(entry: NoteTitleEntry): string | null {
		if (!entry.path) return null;
		// entry.path is vault-relative (e.g. "folder/note.md"); strip .md for the ref.
		return entry.path.replace(/\\/g, '/').replace(/\.md$/, '');
	}

	// Scan the document for wiki-link marks whose visible text has been changed by
	// the user (text ≠ stored title, and the link is not an explicit alias).
	// Each match triggers a note rename and the mark is updated in-place.
	async function checkWikiLinkRenames() {
		if (!editor || !$appConfig?.enable_wiki_links || isLoadingNote) return;
		const origin = captureOrigin();
		if (!canInsert(origin)) return;

		type RenameItem = { pos: number; size: number; mark: any; newTitle: string };
		const toRename: RenameItem[] = [];

		editor.state.doc.descendants((node: any, pos: number) => {
			if (!node.isText) return;
			const wikiMark = node.marks.find((m: any) => m.type.name === 'wikiLink');
			// Skip unresolved links (no path) and explicit aliases
			if (!wikiMark || !wikiMark.attrs.path || wikiMark.attrs.aliased) return;
			const text = (node.text as string || '').trim();
			const storedTitle = (wikiMark.attrs.title as string || '').trim();
			if (text && text !== storedTitle) {
				toRename.push({ pos, size: node.nodeSize, mark: wikiMark, newTitle: text });
			}
		});

		if (toRename.length === 0) return;

		// Save the current note before renaming so the Rust backend has the latest content
		if (!(await forceSave()) || !isOriginCurrent(origin, true)) return;

		for (const item of toRename) {
			try {
				const absPath = resolveNotePath(item.mark.attrs.path);
				const newPath = await renameNote(origin.vault!, absPath, item.newTitle);
				if (!isOriginCurrent(origin, true)) return;
				// Update the mark attrs in the editor (title + path) so the next save
				// serialises [[NewTitle]] and doesn't re-trigger the rename check
				if (editor) {
					const wikiMarkType = editor.schema.marks.wikiLink;
					const tr = editor.state.tr;
					tr.addMark(
						item.pos,
						item.pos + item.size,
						wikiMarkType.create({ title: item.newTitle, path: newPath, aliased: false }),
					);
					ignoreNextUpdate = true;
					editor.view.dispatch(tr);
					ignoreNextUpdate = false;
				}
				refreshWikiLinkTitles();
			} catch (e) {
				console.error('Failed to rename note from wiki-link edit:', e);
			}
		}
		if (isOriginCurrent(origin, true)) markEdited();
	}

	function insertWikiLink(entry: NoteTitleEntry, originalRef?: string) {
		if (!editor || !wikiLinkMenu) return;
		const { from } = wikiLinkMenu;
		const to = editor.state.selection.from;
		// For ambiguous titles, use vault-relative path as the ref so it survives source-mode roundtrips
		const displayText = entry.title;
		let titleAttr = originalRef || entry.title;
		if (entry.path && wikiLinkDuplicateTitles.has(entry.title.toLowerCase())) {
			const relPath = wikiLinkRelPath(entry);
			if (relPath) {
				// Preserve any #heading or ^block anchors from the original ref
				const anchor = originalRef ? originalRef.replace(/^[^#^]*/, '') : '';
				titleAttr = relPath + anchor;
			}
		}
		const transaction = replaceWithWikiLink(
			editor.state,
			from,
			to,
			displayText,
			{ title: titleAttr, path: entry.path || '', aliased: displayText !== titleAttr },
		);
		closeWikiLinkMenu();
		editor.view.dispatch(transaction);
		editor.view.focus();
	}

	function executeWikiLinkCommand(index: number) {
		const items = wikiLinkFiltered;
		if (index < 0 || index >= items.length) return;
		if (wikiLinkDisambigEntries) {
			// In disambiguation mode: use stored display/ref
			insertWikiLink({ ...items[index], title: wikiLinkDisambigDisplay || items[index].title }, wikiLinkDisambigRef || undefined);
		} else {
			insertWikiLink(items[index]);
		}
	}

	const WikiLink = TiptapMark.create({
		name: 'wikiLink',
		// inclusive: true so typing at the end of a link extends the mark, allowing
		// the user to edit a title in-place. Moving the cursor one step past the mark
		// exits it, so typing plain text after a link still works normally.
		inclusive: true,
		// Naming `link` alone would let two wikiLink marks sit on the same text, and
		// updating a link's attrs would then write `[[Old|[[New]]]]`.
		excludes: 'wikiLink link',
		addAttributes() {
			return {
				title: { default: null },
				path: { default: null },
				// aliased=true when the display text was explicitly different from the note
				// title (e.g. [[Note|display]]). Aliased links are never used for rename detection.
				aliased: { default: false },
			};
		},
		parseHTML() {
			// Normalize legacy absolute paths in data-path to vault-relative.
			// Older Windows builds wrote absolute paths (C:\Users\...) into wikilink
			// data-path attributes, which break when the vault is synced to another device.
			const normalizePath = (rawPath: string | null): string | null => {
				if (!rawPath) return null;
				const vaultRoot = $appConfig?.active_vault;
				let norm = rawPath.replace(/\\/g, '/');
				if (vaultRoot) {
					const root = vaultRoot.replace(/\\/g, '/');
					if (norm.startsWith(root + '/')) {
						norm = norm.slice(root.length + 1);
					}
				}
				return norm;
			};
			return [
				{
					tag: 'span[data-wiki-link]',
					getAttrs: (el: HTMLElement) => {
						const title = el.getAttribute('data-title') || null;
						const text = el.textContent || '';
						return {
							title,
							path: normalizePath(el.getAttribute('data-path')),
							// Detect alias from the HTML: if the visible text differs from the
							// stored title the link was serialised as [[ref|display]]
							aliased: el.getAttribute('data-aliased') === '1' || (!!title && text !== title),
						};
					},
				},
				{
					tag: 'a[data-wiki-link]',
					getAttrs: (el: HTMLElement) => {
						const title = el.getAttribute('data-title') || null;
						const text = el.textContent || '';
						return {
							title,
							path: normalizePath(el.getAttribute('data-path')),
							aliased: el.getAttribute('data-aliased') === '1' || (!!title && text !== title),
						};
					},
				},
			];
		},
		renderHTML({ HTMLAttributes }: { HTMLAttributes: Record<string, any> }) {
			const attrs: Record<string, string> = {
				'data-wiki-link': '',
				'data-path': HTMLAttributes.path || '',
				'data-title': HTMLAttributes.title || '',
				class: 'wiki-link',
			};
			if (HTMLAttributes.aliased) attrs['data-aliased'] = '1';
			return ['span', attrs, 0];
		},
		addProseMirrorPlugins() {
			return [
				new Plugin({
					key: new PluginKey('wikiLinkClick'),
					props: {
						handleDOMEvents: {
							click: (view, event) => {
								const target = event.target as HTMLElement;
								const wikiLinkEl = target.closest?.('span[data-wiki-link]') as HTMLElement | null;
								if (wikiLinkEl) {
									event.preventDefault();
									event.stopPropagation();
									const path = wikiLinkEl.getAttribute('data-path') || '';
									// Prefer displayed text over stored title — if the user edited the
									// link text after the note was deleted, textContent reflects the
									// new name they want, while data-title still holds the old one.
									const title = wikiLinkEl.textContent || wikiLinkEl.getAttribute('data-title') || '';
									navigateToWikiLink(path, title, event as MouseEvent);
									return true;
								}
								return false;
							},
						},
					},
				}),
			];
		},
	});

	const WikiLinkAutocomplete = Extension.create({
		name: 'wikiLinkAutocomplete',
		addProseMirrorPlugins() {
			return [
				new Plugin({
					key: new PluginKey('wikiLinkAutocomplete'),
					props: {
						handleKeyDown: (_view, event) => {
							if (!wikiLinkMenu) return false;
							if (event.key === 'ArrowDown') {
								event.preventDefault();
								wikiLinkSelectedIndex = (wikiLinkSelectedIndex + 1) % Math.max(1, wikiLinkFiltered.length);
								return true;
							}
							if (event.key === 'ArrowUp') {
								event.preventDefault();
								wikiLinkSelectedIndex = (wikiLinkSelectedIndex - 1 + wikiLinkFiltered.length) % Math.max(1, wikiLinkFiltered.length);
								return true;
							}
							if (event.key === 'Enter' || event.key === 'Tab') {
								if (wikiLinkFiltered.length > 0) {
									event.preventDefault();
									executeWikiLinkCommand(wikiLinkSelectedIndex);
									return true;
								}
								closeWikiLinkMenu();
								return false;
							}
							if (event.key === 'Escape') {
								event.preventDefault();
								closeWikiLinkMenu();
								return true;
							}
							return false;
						},
						handleTextInput: (view, from, to, text) => {
							if (!$appConfig?.enable_wiki_links) return false;
							// Detect [[ opening: flag so onTransaction opens the menu on mobile
							if (text === '[') {
								const charBefore = from > 0 ? view.state.doc.textBetween(from - 1, from) : '';
								if (charBefore === '[') wikiLinkTypedByUser = true;
							}
							// Detect ]] closing: auto-resolve the current text as a wiki-link
							if (text === ']' && wikiLinkMenu) {
								const state = view.state;
								const textBefore = state.doc.textBetween(wikiLinkMenu.from, state.selection.from);
								if (textBefore.endsWith(']')) {
									// Supports Obsidian syntax: [[note|alias]], [[note#heading]], [[note^block]]
									const rawQuery = textBefore.slice(2, -1); // strip the [[ and trailing ]
									if (rawQuery.trim()) {
										const pipeIdx = rawQuery.indexOf('|');
										const noteRef = (pipeIdx >= 0 ? rawQuery.slice(0, pipeIdx) : rawQuery).trim();
										const display = (pipeIdx >= 0 ? rawQuery.slice(pipeIdx + 1) : noteRef).trim();
										// Strip #heading and ^block for title matching
										const titleForLookup = noteRef.replace(/#.*$/, '').replace(/\^.*$/, '').trim();
										const matches = wikiLinkTitlesCache.filter(e => e.title.toLowerCase() === titleForLookup.toLowerCase());
										if (matches.length === 1) {
											insertWikiLink({ ...matches[0], title: display }, noteRef);
										} else if (matches.length > 1) {
											// Keep the menu open but filter to only the matching entries
											wikiLinkMenu = { ...wikiLinkMenu!, query: titleForLookup };
											// Override filtered results to only show exact matches
											wikiLinkDisambigEntries = matches;
											wikiLinkDisambigRef = noteRef;
											wikiLinkDisambigDisplay = display;
											wikiLinkSelectedIndex = 0;
										} else {
										// Insert as unresolved wiki-link (no path)
										const menuFrom = wikiLinkMenu.from;
										const curTo = state.selection.from;
										closeWikiLinkMenu();
										const origin = captureOrigin();
										tick().then(() => {
											if (!editor || !canInsert(origin) || !isOriginCurrent(origin, true)) return;
											editor.view.dispatch(replaceWithWikiLink(
												editor.state,
												menuFrom,
												curTo,
												display,
												{ title: noteRef, path: '', aliased: display !== noteRef },
											));
										});
										}
									} else {
										closeWikiLinkMenu();
									}
									return true;
								}
							}
							return false;
						},
					},
				}),
			];
		},
	});

	function updateWikiLinkMenu() {
		wikiLinkTypedByUser = false;
		if (!editor || !$appConfig?.enable_wiki_links) return;
		const { state } = editor;
		const { selection } = state;
		const resolvedFrom = selection.$from;
		const parentNode = resolvedFrom.parent;
		if (parentNode.type.name !== 'paragraph' && parentNode.type.name !== 'heading') {
			closeWikiLinkMenu();
			return;
		}
		// Build textBefore from the actual ProseMirror node content so positions are accurate
		// (parentNode.textContent flattens images/atoms, causing position miscalculation)
		let textBefore = '';
		const cursorOffset = resolvedFrom.parentOffset;
		parentNode.forEach((child, offset) => {
			if (offset >= cursorOffset) return false;
			if (child.isText) {
				textBefore += child.text!.slice(0, Math.min(child.nodeSize, cursorOffset - offset));
			}
		});
		// Match [[ — also allow exactly one trailing ] so the menu stays open after
		// the first ] of ]] is typed, letting handleTextInput catch the closing ]]
		const match = textBefore.match(/\[\[([^\]]*)\]?$/);
		if (!match) {
			closeWikiLinkMenu();
			return;
		}
		// Refresh titles when the menu first opens so newly created notes are found
		if (!wikiLinkMenu) refreshWikiLinkTitles();
		const query = match[1];
		// Calculate from as cursor position minus the matched text length ("[[query")
		const from = resolvedFrom.pos - match[0].length;
		const coords = editor.view.coordsAtPos(from);
		let x = coords.left;
		if (x + 280 > window.innerWidth) x = window.innerWidth - 290;
		const vv = window.visualViewport;
		const visibleBottom = isMobile && vv ? vv.height + (vv.offsetTop || 0) : window.innerHeight;
		const menuHeight = 360;
		let y = coords.bottom + 4;
		if (y + menuHeight > visibleBottom) y = coords.top - menuHeight - 4;
		if (y < 4) y = 4;
		wikiLinkMenu = { x, y, query, from };
		wikiLinkSelectedIndex = 0;
	}

	function navigateToWikiLink(path: string, title: string, clickEvent?: MouseEvent) {
		return resolveWikiLinkNavigation(path, title, clickEvent, beginEditorNoteNavigation());
	}

	async function resolveWikiLinkNavigation(path: string, title: string, clickEvent: MouseEvent | undefined, isCurrent: () => boolean) {
		const vault = $appConfig?.active_vault;
		if (!isCurrent() || !vault) return;
		// Normalize legacy absolute paths in data-path to vault-relative (issue: cross-device sync)
		path = normalizeWikiPath(path);
		// title may contain #heading or ^block anchors - strip for note lookup
		const noteTitle = title.replace(/#.*$/, '').replace(/\^.*$/, '').trim();
		if (!path) {
			// Try path-based resolution first (for disambiguated refs like "folder/note")
			if (noteTitle.includes('/')) {
				// entry.path is vault-relative (e.g. "folder/note.md"); match against that
				const relPath = noteTitle + '.md';
				const pathMatch = wikiLinkTitlesCache.find(e => e.path === relPath);
				if (pathMatch) {
					path = pathMatch.path;
				} else {
					const lastSegment = noteTitle.split('/').pop()!;
					const segMatches = wikiLinkTitlesCache.filter(e => e.title.toLowerCase() === lastSegment.toLowerCase());
					if (segMatches.length === 1) path = segMatches[0].path;
					else if (segMatches.length > 1) {
						let x = clickEvent ? clickEvent.clientX : window.innerWidth / 2 - 140;
						let y = clickEvent ? clickEvent.clientY + 8 : window.innerHeight / 2 - 100;
						if (x + 280 > window.innerWidth) x = window.innerWidth - 290;
						if (y + 200 > window.innerHeight) y = Math.max(4, window.innerHeight - 200);
						wikiLinkNavDisambig = { entries: segMatches, x, y };
						wikiLinkNavDisambigIndex = 0;
						return;
					}
				}
			}
		}
		if (!path) {
			const matches = wikiLinkTitlesCache.filter(e => e.title.toLowerCase() === noteTitle.toLowerCase());
			if (matches.length === 1) {
				path = matches[0].path;
			} else if (matches.length > 1) {
				let x = clickEvent ? clickEvent.clientX : window.innerWidth / 2 - 140;
				let y = clickEvent ? clickEvent.clientY + 8 : window.innerHeight / 2 - 100;
				if (x + 280 > window.innerWidth) x = window.innerWidth - 290;
				if (y + 200 > window.innerHeight) y = Math.max(4, window.innerHeight - 200);
				wikiLinkNavDisambig = { entries: matches, x, y };
				wikiLinkNavDisambigIndex = 0;
				return;
			} else {
				// Create the note (use clean title, not the anchor ref)
				const cleanTitle = noteTitle.includes('/') ? noteTitle.split('/').pop()! : noteTitle;
				const notebookRel = $activeNotePath
					? $activeNotePath.replace(/\\/g, '/').replace(($appConfig?.active_vault ?? '').replace(/\\/g, '/') + '/', '').split('/').slice(0, -1).join('/')
					: null;
				try {
					// Save the current note before navigating away
					if (!(await flushSave()) || !isCurrent()) return;
					const { createNote } = await import('$lib/api');
					if (!isCurrent()) return;
					const newNote = await createNote(vault, notebookRel || null, cleanTitle);
					if (!isCurrent()) return;
					// Refresh titles cache so the new note resolves on future clicks
					refreshWikiLinkTitles();
					// Navigate to the new note
					const content = await readNote(vault, newNote.path);
					if (!isCurrent()) return;
					if (!(await flushSave()) || !isCurrent()) return;
					$activeNote = content;
					$activeNotePath = newNote.path;
					$editorDirty = false;
				} catch (e) {
					console.error('Failed to create note from wiki-link:', e);
				}
				return;
			}
		}
		try {
			const absPath = resolveNotePath(path);
			if (absPath === $activeNotePath) return;
			const content = await readNote(vault, absPath);
			if (!isCurrent()) return;
			if (!(await flushSave()) || !isCurrent()) return;
			$activeNote = content;
			$activeNotePath = absPath;
		} catch (e) {
			if (!isCurrent()) return;
			// Explicit Markdown links do not have a wiki-link title to recreate from.
			// Do not turn a failed read into an untitled note.
			if (!noteTitle) {
				console.error('Failed to navigate to note link:', e);
				return;
			}
			// Note at path no longer exists (deleted/moved). Refresh cache and
			// retry as unresolved so the user can recreate it from the link.
			await refreshWikiLinkTitles();
			if (!isCurrent()) return;
			await resolveWikiLinkNavigation('', title, clickEvent, isCurrent);
		}
	}

	async function navigateToWikiLinkDirect(entry: NoteTitleEntry) {
		const isCurrent = beginEditorNoteNavigation();
		const vault = $appConfig?.active_vault;
		if (!isCurrent() || !vault) return;
		wikiLinkNavDisambig = null;
		try {
			const absPath = resolveNotePath(entry.path);
			if (absPath === $activeNotePath) return;
			const content = await readNote(vault, absPath);
			if (!isCurrent()) return;
			if (!(await flushSave()) || !isCurrent()) return;
			$activeNote = content;
			$activeNotePath = absPath;
		} catch (e) {
			console.error('Failed to navigate to wiki-link:', e);
		}
	}

	const textColors = [
		{ name: 'Default', value: '' },
		{ name: 'Red', value: '#ef4444' },
		{ name: 'Orange', value: '#f97316' },
		{ name: 'Amber', value: '#f59e0b' },
		{ name: 'Green', value: '#22c55e' },
		{ name: 'Blue', value: '#3b82f6' },
		{ name: 'Purple', value: '#a855f7' },
		{ name: 'Pink', value: '#ec4899' },
	];

	const highlightColors = [
		{ name: 'Yellow', value: 'rgba(250, 230, 100, 0.25)', swatch: '#f5e050' },
		{ name: 'Green', value: 'rgba(100, 210, 130, 0.22)', swatch: '#5cc870' },
		{ name: 'Blue', value: 'rgba(100, 170, 240, 0.22)', swatch: '#6aabf0' },
		{ name: 'Purple', value: 'rgba(180, 130, 240, 0.22)', swatch: '#a878e8' },
		{ name: 'Pink', value: 'rgba(240, 140, 180, 0.22)', swatch: '#e88aaa' },
		{ name: 'Red', value: 'rgba(240, 120, 120, 0.22)', swatch: '#e07070' },
		{ name: 'Orange', value: 'rgba(240, 170, 90, 0.25)', swatch: '#e8a050' },
		{ name: 'Cyan', value: 'rgba(80, 210, 230, 0.22)', swatch: '#50cce0' },
	];

	const cellColors = [
		{ name: 'None', value: '' },
		{ name: 'Light Red', value: '#fde8e8' },
		{ name: 'Light Orange', value: '#fef3e2' },
		{ name: 'Light Yellow', value: '#fef9e7' },
		{ name: 'Light Green', value: '#e6f8e0' },
		{ name: 'Light Blue', value: '#e0f0fe' },
		{ name: 'Light Purple', value: '#f0e6fe' },
		{ name: 'Light Pink', value: '#fde8f0' },
		{ name: 'Light Gray', value: '#f3f4f6' },
		{ name: 'Dark Red', value: '#7f1d1d' },
		{ name: 'Dark Amber', value: '#713f12' },
		{ name: 'Dark Green', value: '#14532d' },
		{ name: 'Dark Blue', value: '#1e3a5f' },
		{ name: 'Dark Purple', value: '#4c1d95' },
		{ name: 'Dark Pink', value: '#831843' },
		{ name: 'Dark Teal', value: '#064e3b' },
		{ name: 'Dark Cyan', value: '#0c4a6e' },
		{ name: 'Slate', value: '#1e293b' },
		{ name: 'Gray', value: '#374151' },
	];

	function currentNoteAssetRoot(): string | null {
		return noteAssetRoot($activeNotePath, $appConfig?.active_vault ?? null, $appConfig?.vaults.map(vault => vault.path) ?? []);
	}

	function resolveImageSrc(src: string): string {
		// Already a proxied, asset, data, or blob URL
		if (src.startsWith('data:') || src.startsWith('asset:') || src.startsWith('blob:') || src.startsWith('imgproxy:') || src.startsWith('http://imgproxy.localhost') || src.startsWith('https://imgproxy.localhost')) {
			return src;
		}
		// Already an asset-localhost URL
		if (src.startsWith('http://asset.localhost') || src.startsWith('https://asset.localhost')) {
			return src;
		}
		// External http/https URLs: proxy through Tauri's imgproxy protocol
		// to bypass WebKitGTK restrictions on loading external resources
		if (src.startsWith('http://') || src.startsWith('https://')) {
			return convertFileSrc(src, 'imgproxy');
		}
		const local = resolveFileReference(src, $activeNotePath, currentNoteAssetRoot());
		return convertFileSrc(normalizePath(local.path)) + local.suffix;
	}

	function normalizeImageHtml(html: string): string {
		if (!/<img\b/i.test(html)) return html;
		// Template contents are inert: parsing raw image HTML must not fetch its
		// original URL before Tiptap gets a chance to normalize the image node.
		const template = document.createElement('template');
		template.innerHTML = html;
		for (const image of template.content.querySelectorAll('img')) {
			for (const attribute of Array.from(image.attributes)) {
				if (!['src', 'alt', 'title', 'data-size', 'width', 'height'].includes(attribute.name)) {
					image.removeAttribute(attribute.name);
				}
			}
			const source = image.getAttribute('src');
			if (source) image.setAttribute('src', resolveImageSrc(source));
			const size = (image.getAttribute('alt') ?? '').match(/^(.*?)\|size=(small|medium|full)$/);
			if (size) {
				image.setAttribute('alt', size[1]);
				image.setAttribute('data-size', size[2]);
			}
		}
		return template.innerHTML;
	}

	const saves = new RevisionSaveQueue();
	let savedBlobImages = new Map<string, string>();
	let saveError = $state<{ message: string; path: string; vault: string; conflict: boolean } | null>(null);
	let saveAttempt = 0;
	let retryingSave = $state(false);

	type EditorOrigin = {
		snapshot: SaveRevision;
		path: string | null;
		vault: string | null | undefined;
		noteId: string | undefined;
		sourceMode: boolean;
		surface: Editor | null;
	};

	function captureOrigin(): EditorOrigin {
		return { snapshot: saves.capture(), path: $activeNotePath, vault: $appConfig?.active_vault,
			noteId: $activeNote?.meta.id, sourceMode: $sourceMode, surface: editor };
	}

	function isOriginCurrent(origin: EditorOrigin, unchanged = false): boolean {
		return !editorDisposed && !!origin.path && origin.path === loadedPath
			&& origin.path === $activeNotePath && origin.vault === $appConfig?.active_vault
			&& origin.sourceMode === $sourceMode && (origin.sourceMode || origin.surface === editor)
			&& (unchanged ? saves.isCurrent(origin.snapshot) : saves.isCurrentSession(origin.snapshot));
	}

	function canInsert(origin: EditorOrigin): boolean {
		return !externalOperationBusy && !pathMigrationPending && isOriginCurrent(origin) && !!origin.vault && !trashPreview && !$readOnly && !$viewerNote;
	}

	export function markEdited(): void {
		saves.markChanged();
		$editorDirty = true;
		saves.schedule(() => { void forceSave(); });
	}

	function isCurrentSaveAttempt(origin: EditorOrigin, attempt: number): boolean {
		return !editorDisposed && attempt === saveAttempt && saves.isCurrentSession(origin.snapshot)
			&& origin.path === $activeNotePath && origin.vault === $appConfig?.active_vault;
	}

	function reportSaveError(error: unknown, origin: EditorOrigin, attempt: number): false {
		console.error('Save failed:', error);
		if (origin.path && origin.vault && isCurrentSaveAttempt(origin, attempt)) {
			const message = error instanceof Error ? error.message : String(error);
			saveError = { message: message.trim() || 'The note could not be saved.', path: origin.path, vault: origin.vault, conflict: /conflict|modified outside/i.test(message) };
		}
		return false;
	}

	async function retrySave() {
		if (retryingSave || !saveError || saveError.path !== $activeNotePath || saveError.vault !== $appConfig?.active_vault) return;
		const snapshot = saves.capture();
		retryingSave = true;
		try {
			await forceSave();
		} finally {
			if (saves.isCurrentSession(snapshot)) retryingSave = false;
		}
	}

	async function saveConflictCopy() {
		if (!saveError?.conflict || !$activeNote || !$activeNotePath || !$appConfig?.active_vault || retryingSave) return;
		const origin = captureOrigin();
		const vault = origin.vault!;
		const originalPath = origin.path!;
		let body = getCurrentBody();
		const meta = { ...$activeNote.meta, tags: [...$activeNote.meta.tags] };
		retryingSave = true;
		try {
			await saves.idle();
			if (!isOriginCurrent(origin, true)) return;
			const parent = originalPath.slice(vault.length + 1).replace(/[/\\][^/\\]+$/, '');
			const notebook = parent === originalPath.slice(vault.length + 1) ? null : parent.replace(/\\/g, '/');
			const entry = await createNote(vault, notebook, `${meta.title} (conflict copy)`);
			for (const match of body.matchAll(/blob:[^)\s"']+/g)) {
				const url = match[0];
				let relativePath = savedBlobImages.get(url);
				if (!relativePath) {
					relativePath = await saveBlobImage(vault, url);
					savedBlobImages.set(url, relativePath);
				}
				body = body.replaceAll(url, relativePath);
			}
			const copy = await readNote(vault, entry.path);
			await saveNote(vault, entry.path, { ...copy.meta, pinned: meta.pinned, tags: meta.tags }, body, copy.raw);
			if (!isOriginCurrent(origin, true)) return;
			const current = await readNote(vault, entry.path);
			if (!isOriginCurrent(origin, true)) return;
			$activeNotePath = entry.path;
			$activeNote = current;
			loadNote(entry.path, current.content);
		} catch (error) {
			console.error('Failed to save conflict copy:', error);
		} finally {
			retryingSave = false;
		}
	}

	async function reloadConflictedNote() {
		if (!saveError?.conflict || !$activeNotePath || !$appConfig?.active_vault) return;
		if (!window.confirm('Discard your unsaved draft and reload this note from disk? Choose Save a copy to keep the draft.')) return;
		const origin = captureOrigin();
		try {
			await saves.idle();
			if (!isOriginCurrent(origin, true)) return;
			const content = await readNote(origin.vault!, origin.path!);
			if (!isOriginCurrent(origin, true)) return;
			$activeNote = content;
			loadNote(origin.path!, content.content);
		} catch (error) {
			console.error('Failed to reload conflicted note:', error);
		}
	}

	export function forceSave(allowWhileTrashing = false): Promise<boolean> {
		saves.cancelScheduled();
		if (externalOperationBusy || pathMigrationPending) return Promise.resolve(false);
		if (get(viewerNote) || !$activeNote || !$activeNotePath) return Promise.resolve(true);
		if ((renamingNote && saves.isCurrentSession(renamingNote)) || (trashingNote && !allowWhileTrashing)) return Promise.resolve(false);
		if (!$editorDirty) return Promise.resolve(true);
		// Refuse an uninitialized/stale surface, not an intentionally deleted body.
		if ($activeNotePath !== loadedPath || (!$sourceMode && (!editor || pendingContent !== null))) {
			return Promise.resolve(false);
		}
		const origin = captureOrigin();
		const attempt = ++saveAttempt;
		try {
			// Capture everything before yielding: queued work must never read another note's state.
			const { snapshot } = origin;
			const path = $activeNotePath;
			const vault = origin.vault;
			if (!vault) return Promise.resolve(false);
			const meta = { ...$activeNote.meta, tags: [...$activeNote.meta.tags] };
			let body = $sourceMode ? restoreTitleH1(source.state.doc.toString()) : editorToMarkdown();
			const blobUrls = new Set<string>();
			if (body.includes('blob:')) {
				if ($sourceMode) {
					for (const match of body.matchAll(/(?:!\[[^\]\n]*\]\(|src=["'])(blob:[^)\s"']+)/g)) {
						blobUrls.add(match[1]);
					}
				} else {
					editor!.state.doc.descendants((node) => {
						if (node.type.name === 'image' && node.attrs.src?.startsWith('blob:')) {
							blobUrls.add(node.attrs.src);
						}
					});
				}
			}
			const blobImages = savedBlobImages;
			let savedContent: NoteContent | null = null;
			return saves.save(snapshot, async (expectedRaw) => {
				for (const url of blobUrls) {
					let relativePath = blobImages.get(url);
					if (!relativePath) {
						relativePath = await saveBlobImage(vault, url);
						blobImages.set(url, relativePath);
					}
					body = body.replaceAll(url, relativePath);
					replaceSavedBlob(origin, url, relativePath);
				}
				savedContent = await saveNote(vault, path, meta, body, expectedRaw);
				return savedContent.raw;
			}).then((current) => {
				if (isCurrentSaveAttempt(origin, attempt)) saveError = null;
				if (!current || !isOriginCurrent(origin, true)) return false;
				if (savedContent) $activeNote = savedContent;
				$editorDirty = false;
				return true;
			}).catch((error) => reportSaveError(error, origin, attempt));
		} catch (e) {
			return Promise.resolve(reportSaveError(e, origin, attempt));
		}
	}

	export async function moveOpenNoteToTrash(): Promise<boolean> {
		const moveToTrash = onMoveToTrash;
		if (trashingNote || !$activeNotePath || $viewerNote || !moveToTrash) return false;
		const path = $activeNotePath;
		const origin = captureOrigin();
		trashingNote = true;
		try {
			// Capture the latest editor state before waiting for older saves.
			if (!(await forceSave(true)) || !isOriginCurrent(origin)) return false;
			await saves.idle();
			if (!isOriginCurrent(origin, true) || $editorDirty) return false;
			// A queued debounce observes this flag and cannot recreate the moved note.
			$editorDirty = false;
			return await moveToTrash(path);
		} finally {
			trashingNote = false;
		}
	}

	// The markdown body the editor would currently save; used to ignore the file-watcher echo of our own save.
	export function getCurrentBody(): string {
		return $sourceMode ? restoreTitleH1(source.state.doc.toString()) : editorToMarkdown();
	}

	export function getAcceptedRaw(): string {
		return saves.raw;
	}

	export function flagExternalConflict() {
		if (!$editorDirty || !$activeNotePath || !$appConfig?.active_vault) return;
		saveError = { path: $activeNotePath, vault: $appConfig.active_vault,
			message: 'The file changed on disk. Save a copy of your draft or explicitly reload the disk version.',
			conflict: true };
	}

	export function isMutationPending(): boolean {
		return externalOperationBusy || pathMigrationPending;
	}

	// ── Tag editing (active note) ──
	function toggleTagMenu(e: MouseEvent) {
		if (trashPreview) return;
		if (tagMenu) { tagMenu = null; return; }
		const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
		const width = 240;
		const x = Math.max(8, Math.min(rect.left, window.innerWidth - width - 8));
		tagMenu = { x, y: rect.bottom + 4 };
	}

	function addActiveNoteTag(tag: string) {
		if (!$activeNote || trashPreview) return;
		const cleaned = tag.trim().toLowerCase();
		if (!cleaned || $activeNote.meta.tags.includes(cleaned)) return;
		$activeNote = { ...$activeNote, meta: { ...$activeNote.meta, tags: [...$activeNote.meta.tags, cleaned] } };
		markEdited();
	}

	function removeActiveNoteTag(tag: string) {
		if (!$activeNote || trashPreview) return;
		$activeNote = { ...$activeNote, meta: { ...$activeNote.meta, tags: $activeNote.meta.tags.filter((t) => t !== tag) } };
		markEdited();
	}

	// Sync editor editable state when readOnly store changes (from titlebar or editor)
	$effect(() => {
		if (trashPreview && !$readOnly) $readOnly = true;
		const ro = $readOnly;
		const inspecting = showHistory;
		untrack(() => {
			if (editor) {
				if (ro && $editorDirty) forceSave();
				editor.setEditable(!ro && !inspecting && !externalOperationBusy);
			}
		});
	});

	// Belt-and-suspenders: viewer mode is always read-only, regardless of any other path
	// that might toggle setEditable. Re-asserts every time the editor or viewer state changes.
	$effect(() => {
		const v = $viewerNote;
		untrack(() => {
			if (editor && v) editor.setEditable(false);
		});
	});

	function countWords(text: string): number {
		return text.trim() ? text.trim().split(/\s+/).filter(Boolean).length : 0;
	}

	function updateCounts() {
		if (!showInfo || editorDisposed) return;
		let text = '';
		if (get(sourceMode)) {
			text = source.state.doc.toString()
				.replace(/^---[\s\S]*?---\n?/, '')
				.replace(/!\[[^\]]*\]\([^)]*\)/g, '')
				.replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
				.replace(/^#{1,6}\s+/gm, '')
				.replace(/[*_~`]+/g, '')
				.replace(/^>\s*/gm, '')
				.replace(/^[-*+]\s+/gm, '');
		} else if (editor) {
			text = editor.state.doc.textContent;
		}
		wordCount = countWords(text);
		charCount = text.replace(/\s/g, '').length;
	}

	let countsTimer: ReturnType<typeof setTimeout> | undefined;
	function cancelCounts() {
		clearTimeout(countsTimer);
		countsTimer = undefined;
	}
	function scheduleCounts() {
		cancelCounts();
		if (showInfo) countsTimer = setTimeout(updateCounts, 250);
	}

	function toggleInfo() {
		showInfo = !showInfo;
	}

	async function copyActiveNotePath() {
		const path = $activeNotePath;
		if (!path) return;
		if (infoPathCopyTimer) clearTimeout(infoPathCopyTimer);
		try {
			await copyTextToClipboard(path);
			infoPathCopyState = 'copied';
		} catch (e) {
			console.error('Failed to copy note path:', e);
			infoPathCopyState = 'error';
		}
		infoPathCopyTimer = setTimeout(() => {
			infoPathCopyState = 'idle';
			infoPathCopyTimer = null;
		}, 1600);
	}

	let historyRequest = 0;

	function closeHistory() {
		historyRequest++;
		showHistory = false;
		historyVersions = [];
		historySelected = null;
		historyPreview = null;
		historyLoading = false;
		historyError = null;
		historyPreviewLoading = false;
		historyPreviewError = null;
	}

	async function toggleHistory() {
		if (showHistory) {
			closeHistory();
			return;
		}
		closeNoteSearch(false);
		endToolbarCustomization();
		historyWorking = $editorDirty
			? ($sourceMode ? restoreTitleH1(source.state.doc.toString()) : editorToMarkdown())
			: ($activeNote?.content ?? '');
		showHistory = true;
		await loadHistory();
	}

	async function loadHistory() {
		const origin = captureOrigin();
		const request = ++historyRequest;
		const current = () => !editorDisposed && request === historyRequest && showHistory;
		historyVersions = [];
		historySelected = null;
		historyPreview = null;
		historyPreviewLoading = false;
		historyPreviewError = null;
		historyError = null;
		historyLoading = false;
		if (!origin.noteId || !origin.vault) {
			historyError = 'Version history is unavailable for this note.';
			return;
		}
		historyLoading = true;
		try {
			const versions = await getNoteVersions(origin.vault, origin.noteId);
			if (!current()) return;
			if (!isOriginCurrent(origin)) {
				historyError = 'The editor changed while versions were loading. Retry to reload them.';
				return;
			}
			historyVersions = versions;
			historyLoading = false;
			if (versions[0]) void previewVersion(versions[0]);
		} catch (e) {
			console.error('Failed to load versions:', e);
			if (current()) historyError = `Could not load versions: ${String(e)}`;
		} finally {
			if (current()) historyLoading = false;
		}
	}

	async function previewVersion(v: VersionEntry) {
		const origin = captureOrigin();
		if (!origin.noteId || !origin.vault) return;
		const request = ++historyRequest;
		const current = () => !editorDisposed && request === historyRequest && showHistory;
		historySelected = v;
		historyPreview = null;
		historyPreviewError = null;
		historyPreviewLoading = true;
		try {
			const content = await getNoteVersionContent(origin.vault, origin.noteId, v.timestamp);
			if (!current()) return;
			if (!isOriginCurrent(origin)) {
				historyPreviewError = 'The editor changed while this version was loading. Select it again to retry.';
				return;
			}
			historyPreview = content;
		} catch (e) {
			console.error('Failed to load version:', e);
			if (current()) historyPreviewError = `Could not load this version: ${String(e)}`;
		} finally {
			if (current()) historyPreviewLoading = false;
		}
	}

	function restoreVersion() {
		if (!$activeNote || !$activeNotePath || !historySelected || historyPreview === null || historyPreviewLoading) return;
		if (!canInsert(captureOrigin())) return;
		if ($editorDirty && !window.confirm('Restore this version and replace your unsaved draft?')) return;
		try {
			const body = splitSnapshot(historyPreview).body;
			if ($sourceMode) {
				source.replace(stripTitleH1(body));
			} else if (editor) {
				editor.commands.setContent(markdownToHtml(body));
			}
			closeHistory();
		} catch (e) {
			console.error('Failed to restore version:', e);
		}
	}

	async function handleCreateVersion() {
		if (!$activeNote || !$activeNotePath) return;
		const origin = captureOrigin();
		const request = historyRequest;
		if (!origin.vault || !(await forceSave()) || !isOriginCurrent(origin, true)) return;
		try {
			await createVersion(origin.vault, origin.path!, origin.noteId!);
			if (!isOriginCurrent(origin) || request !== historyRequest || !showHistory) return;
			const versions = await getNoteVersions(origin.vault, origin.noteId!);
			if (isOriginCurrent(origin) && request === historyRequest && showHistory) historyVersions = versions;
		} catch (e) {
			console.error('Failed to create version:', e);
		}
	}

	const historyDateFormatter = new Intl.DateTimeFormat(undefined, {
		year: 'numeric', month: 'short', day: 'numeric',
		hour: '2-digit', minute: '2-digit', second: '2-digit', fractionalSecondDigits: 3,
	});

	function formatVersionDate(iso: string): string {
		const date = new Date(iso);
		return Number.isNaN(date.getTime()) ? iso : historyDateFormatter.format(date);
	}

	function formatVersionSize(bytes: number): string {
		if (bytes < 1024) return `${bytes} B`;
		return `${(bytes / 1024).toFixed(1)} KB`;
	}

	function clearEditorHistory() {
		if (!editor) return;
		// Recreate editor state with same doc/schema/plugins but fresh plugin state (clears undo/redo)
		const { doc, schema, plugins } = editor.state;
		const newState = EditorState.create({ doc, schema, plugins });
		editor.view.updateState(newState);
	}

	export function focusTitle(navigationToken: number) {
		const vault = $appConfig?.active_vault;
		const path = $activeNotePath;
		tick().then(() => {
			if (editorDisposed || !isCurrentNoteNavigation(navigationToken)
				|| vault !== $appConfig?.active_vault || path !== $activeNotePath) return;
			if (titleInput) {
				titleInput.focus();
				titleInput.select();
			}
		});
	}

	async function editorNavigateHistory(direction: -1 | 1) {
		const isCurrent = beginEditorNoteNavigation();
		const vault = $appConfig?.active_vault;
		if (!isCurrent() || !vault) return;
		if (!(await flushSave()) || !isCurrent()) return;
		const path = navHistory.go(direction);
		if (!path) return;
		try {
			const content = await readNote(vault, path);
			if (!isCurrent()) return;
			if (!(await flushSave()) || !isCurrent()) return;
			$activeNote = content;
			$activeNotePath = path;
			$editorDirty = false;
		} catch {}
	}

	/** Capture and queue unsaved content before changing active-note stores.
	 * A false result means the save failed or newer edits still need saving. */
	export function flushSave(): Promise<boolean> {
		return forceSave();
	}

	/** Keep both editing surfaces stable while a native task mutation edits this note. */
	export async function applyNativeTaskChange(path: string, operation: () => Promise<void>): Promise<void> {
		if (externalOperationBusy || path !== $activeNotePath || !(await flushSave())) return;
		if (externalOperationBusy || $editorDirty || path !== $activeNotePath || !$appConfig?.active_vault) return;
		const vault = $appConfig.active_vault;
		const snapshot = saves.capture();
		externalOperationBusy = true;
		source.configure(true, !isMobile && ($appConfig?.show_line_numbers ?? false));
		editor?.setEditable(false);
		try {
			await operation();
			const content = await readNote(vault, path);
			if (path === $activeNotePath && vault === $appConfig?.active_vault) {
				if (saves.isCurrent(snapshot) && !$editorDirty) {
					$activeNote = content;
					loadNote(path, content.content);
				} else {
					flagExternalConflict();
				}
			}
		} finally {
			externalOperationBusy = false;
			source.configure($readOnly || !!$viewerNote, !isMobile && ($appConfig?.show_line_numbers ?? false));
			editor?.setEditable(!$readOnly && !$viewerNote);
		}
	}

	export async function beginPathMutation(): Promise<(() => void) | null> {
		if (externalOperationBusy) return null;
		const path = $activeNotePath;
		if (!(await flushSave()) || externalOperationBusy || path !== $activeNotePath) return null;
		externalOperationBusy = true;
		source.configure(true, !isMobile && ($appConfig?.show_line_numbers ?? false));
		editor?.setEditable(false);
		return () => {
			externalOperationBusy = false;
			source.configure($readOnly || !!$viewerNote, !isMobile && ($appConfig?.show_line_numbers ?? false));
			editor?.setEditable(!$readOnly && !$viewerNote);
		};
	}

	function rememberLoadedNoteScroll() {
		if (!loadedPath) return;
		const previous = noteScrollPositions.get(loadedPath) ?? { rich: 0, source: 0 };
		const editorBody = editorElement?.closest('.editor-body') as HTMLElement | null;
		const position = $sourceMode
			? { ...previous, source: source.view?.scrollDOM.scrollTop ?? previous.source }
			: { ...previous, rich: editorBody?.scrollTop ?? previous.rich };

		// Refresh insertion order so the cap evicts the least recently viewed note.
		noteScrollPositions.delete(loadedPath);
		noteScrollPositions.set(loadedPath, position);
		if (noteScrollPositions.size > MAX_NOTE_SCROLL_POSITIONS) {
			const oldestPath = noteScrollPositions.keys().next().value;
			if (oldestPath) noteScrollPositions.delete(oldestPath);
		}
	}

	function restoreNoteScroll(surface: HTMLElement, scrollTop: number, path: string) {
		const snapshot = saves.capture();
		const apply = () => {
			if (!searchPassageActive && saves.isCurrentSession(snapshot) && loadedPath === path && surface.isConnected) surface.scrollTop = scrollTop;
		};
		apply();
		requestAnimationFrame(apply);
	}

	/**
	 * Converting or showing a note failed, so the editor still holds the previous note while every
	 * other piece of state already points at this one. Nothing may be saved from it: forget the
	 * loaded path (saving and inserting both require it), empty the view and lock it.
	 */
	function failNoteLoad(path: string, error: unknown) {
		console.error('Failed to open note in the editor:', error);
		loadedPath = '';
		$editorDirty = false;
		saves.cancelScheduled();
		isLoadingNote = false;
		if (editor) {
			ignoreNextUpdate = true;
			try { editor.commands.clearContent(); } catch { /* Saving is already blocked. */ }
			ignoreNextUpdate = false;
			clearEditorHistory();
			editor.setEditable(false);
		}
		loadFailure = path;
	}

	export function loadNote(path: string, content: string, taskTarget?: TaskRecord) {
		loadFailure = null;
		cancelProgressiveLoad();
		endToolbarCustomization();
		clearOutline();
		cancelCounts();
		closeNoteSearch(false);
		saves.startSession($activeNote?.raw ?? content);
		saveError = null;
		retryingSave = false;
		closeAiMenu();
		closeHistory();
		secretModal = null;
		linkModal = false;
		textContextMenu = null;
		closeSlashMenu();
		closeWikiLinkMenu();
		closeTaskMetaMenu();
		taskDuePicker = null;
		codeLangDropdown = null;
		linkContextMenu = null;
		imageToolbar = null;
		tableContextMenu = null;
		savedBlobImages = new Map();
		$editorDirty = false;
		rememberLoadedNoteScroll();
		const scrollPosition = taskTarget ? undefined : noteScrollPositions.get(path);
		clearTargetReveal();
		const revealRequest = ++targetRevealRequest;
		const revealTarget = taskTarget ? resolveTaskTarget(taskTarget, content) : null;
		loadedPath = path;
		const loadSnapshot = saves.capture();
		if ($appConfig?.enable_wiki_links && wikiTitleVault !== $appConfig.active_vault) void refreshWikiLinkTitles();
		isLoadingNote = true;
		isLargeDoc = content.length > LARGE_DOC_CHARS;
		// Viewer mode (external file) always forces read-only. New notes stay editable and
		// can opt into source mode without changing how existing notes choose their mode.
		const isViewer = !!get(viewerNote);
		const isNewNote = $activeNote?.meta.title === 'Untitled' && !content.replace(/^---[\s\S]*?---\s*/, '').trim();
		if (isNewNote && ($appConfig?.new_notes_in_source_mode ?? false)) {
			$sourceMode = true;
		}
		lastSourceMode = $sourceMode;
		const shouldBeReadOnly = isViewer || isTrashedNote(path, $appConfig?.active_vault)
			? true : (isNewNote ? false : ($appConfig?.default_view_mode ?? false));
		$readOnly = shouldBeReadOnly;
		if (editor) editor.setEditable(!shouldBeReadOnly && !externalOperationBusy);
		const editorBody = editorElement?.closest('.editor-body') as HTMLElement | null;
		if ($sourceMode) {
			source.load(stripTitleH1(content));
			if (editorBody) editorBody.scrollTop = 0;
			tick().then(() => {
				if (saves.isCurrentSession(loadSnapshot) && source.view) restoreNoteScroll(source.view.scrollDOM, scrollPosition?.source ?? 0, path);
			});
			isLoadingNote = false;
			updateCounts();
		} else if (editorElement && editor) {
			// Editor already exists, just swap content
			try {
				const html = markdownToHtml(content);
				// Retire the outgoing native selection before its DOM is replaced, or the webview can keep
				// painting the old caret. Selections in other controls are left alone.
				const editorDom = editor.view.dom;
				const domSelection = editorDom.ownerDocument.getSelection();
				if (domSelection && (editorDom.contains(domSelection.anchorNode) || editorDom.contains(domSelection.focusNode))) {
					domSelection.removeAllRanges();
				}
				const settle = () => {
					updateCounts();
					// Restore scroll and reset the cursor after all ProseMirror/Svelte DOM updates settle.
					tick().then(() => {
						if (!saves.isCurrentSession(loadSnapshot) || path !== loadedPath) return;
						if (editorBody) restoreNoteScroll(editorBody, scrollPosition?.rich ?? 0, path);
						// Explicitly reset ProseMirror selection to start so TipTap's focus()
						// (triggered by checkbox clicks etc.) doesn't scroll to the old note's cursor position.
						if (editor) {
							const tr = editor.state.tr.setSelection(TextSelection.atStart(editor.state.doc));
							// No tr.scrollIntoView() - must not trigger any scroll
							editor.view.dispatch(tr);
						}
						isLoadingNote = false;
					});
				};
				if (content.length > LARGE_DOC_CHARS) {
					// Very large note: show the start at once and build the rest in slices.
					loadDocumentProgressively(html, () => {
						settle();
						if (showOutline) updateOutline();
						scheduleTaskReveal(path, revealTarget, revealRequest);
					});
					return;
				}
				// An identical document emits no update, so the flag is cleared here, not by the update
				// handler: left set, it would swallow the user's next edit.
				ignoreNextUpdate = true;
				editor.commands.setContent(html);
				ignoreNextUpdate = false;
				// Clear undo/redo history so it doesn't bleed across notes
				clearEditorHistory();
				settle();
			} catch (error) {
				failNoteLoad(path, error);
				return;
			}
		} else {
			// Editor element not in DOM yet (first note load).
			// Store content and let the $effect on editorElement handle init.
			pendingContent = content;
			isLoadingNote = false;
		}
		if (showOutline) updateOutline();
		scheduleTaskReveal(path, revealTarget, revealRequest);
	}

	// ---- Progressive loading of very large notes
	// Building the page for a few hundred KB of Markdown in one go freezes it for about half a second.
	// Instead the document is built in slices, the first screen at once. Until it is complete the
	// document is partial: the guard plugin rejects edits to it, and any interaction with the editor,
	// or anything that serializes it, first finishes the build synchronously.
	const FIRST_SLICE_BLOCKS = 40;
	const SLICE_BLOCKS = 150;
	const LOAD_INTERRUPTS = ['keydown', 'beforeinput', 'compositionstart', 'paste', 'cut', 'drop', 'pointerdown'] as const;
	let progressiveLoad: { finish: () => void; cancel: () => void } | null = null;

	function cancelProgressiveLoad() {
		progressiveLoad?.cancel();
	}

	function finishProgressiveLoad() {
		progressiveLoad?.finish();
	}

	function loadDocumentProgressively(html: string, whenLoaded: () => void) {
		const view = editor!.view;
		const schema = editor!.schema;
		const plugins = editor!.state.plugins;
		const nodes = Array.from(new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html').body.childNodes);
		const parser = PMDOMParser.fromSchema(schema);
		const blocks: ProseMirrorNode[] = [];
		let next = 0;
		let timer: ReturnType<typeof setTimeout> | undefined;
		let rendered: ProseMirrorNode | null = null;

		let dirty = false;
		const addSlice = (count: number) => {
			dirty = true;
			const container = document.createElement('div');
			for (const node of nodes.slice(next, next + count)) container.append(node);
			next += count;
			parser.parse(container).forEach((block) => blocks.push(block));
		};
		const render = () => {
			if (!dirty) return;
			dirty = false;
			// updateState does not run plugins' appendTransaction, so no stray trailing paragraph appears mid-note.
			view.updateState(EditorState.create({ doc: schema.topNodeType.create(null, blocks), plugins }));
			rendered = view.state.doc;
		};
		const interrupt = (event: Event) => {
			// Only interaction with the editor itself (its text, toolbar and menus) counts, not choosing another note.
			if (event.target instanceof Element && event.target.closest('.editor-container')) finish();
		};
		const stop = () => {
			clearTimeout(timer);
			for (const type of LOAD_INTERRUPTS) window.removeEventListener(type, interrupt, true);
			if (progressiveLoad === handle) progressiveLoad = null;
		};
		const finish = () => {
			if (progressiveLoad !== handle) return;
			if (view.state.doc === rendered && !view.isDestroyed) {
				while (next < nodes.length) addSlice(SLICE_BLOCKS);
				render();
			}
			stop();
			// TipTap normalizes a new document on its first transaction (a trailing paragraph); run it now.
			ignoreNextUpdate = true;
			if (!view.isDestroyed) view.dispatch(view.state.tr);
			ignoreNextUpdate = false;
			whenLoaded();
		};
		const step = () => {
			if (progressiveLoad !== handle) return;
			if (view.state.doc !== rendered) { finish(); return; }  // something else replaced the document
			// Each render re-walks everything already built, so slices double in size: few renders, no long freeze.
			addSlice(Math.max(SLICE_BLOCKS, next));
			render();
			if (next < nodes.length) timer = setTimeout(step, 0); else finish();
		};
		const handle = { finish, cancel: stop };
		progressiveLoad = handle;
		addSlice(FIRST_SLICE_BLOCKS);
		render();
		for (const type of LOAD_INTERRUPTS) window.addEventListener(type, interrupt, true);
		if (next < nodes.length) timer = setTimeout(step, 0); else finish();
	}

	function stripTitleH1(md: string): string {
		const result = stripTitleHeading(md, $activeNote?.meta.title, $appConfig?.hide_title_in_body ?? false);
		hiddenTitleHeading = result.hiddenTitle;
		return result.markdown;
	}

	function restoreTitleH1(md: string): string {
		return restoreTitleHeading(md, hiddenTitleHeading);
	}

	/**
	 * With the title heading shown in the body, the heading and the title field are the same text.
	 * When the field changes, the leading heading that still reads as the old title follows it;
	 * left behind, it was saved under a second heading carrying the new title.
	 */
	function syncVisibleTitleHeading(previous: string, next: string) {
		if (hiddenTitleHeading || !next.trim()) return;
		const same = (text: string) => text.trim().toLowerCase() === previous.trim().toLowerCase();
		if ($sourceMode) {
			const text = source.state.doc.toString();
			const heading = /^(\s*# )([^\n]*)/.exec(text);
			if (heading && same(heading[2])) source.replace(next.trim(), heading[1].length, heading[0].length);
			return;
		}
		if (!editor) return;
		finishProgressiveLoad();
		const first = editor.state.doc.firstChild;
		if (first?.type.name !== 'heading' || first.attrs.level !== 1 || !same(first.textContent)) return;
		editor.view.dispatch(editor.state.tr.insertText(next.trim(), 1, 1 + first.content.size));
	}

	function editorToMarkdown(): string {
		if (!editor) return '';
		finishProgressiveLoad();
		const md = prosemirrorToMarkdown(editor.state.doc);
		return restoreTitleH1(md);
	}

	function isImageNode(node: any): boolean {
		if (node.type.name === 'image') return true;
		if (node.type.name !== 'paragraph') return false;
		let hasImage = false;
		let hasOther = false;
		node.forEach((child: any) => {
			if (child.type.name === 'image') hasImage = true;
			else hasOther = true;
		});
		return hasImage && !hasOther;
	}

	// Saving used to serialize every block of the note each time. Document nodes are immutable and
	// an edit replaces only the blocks it touches, so the Markdown of each top-level block is kept
	// and a save re-serializes just what changed. The cache is dropped when the note or vault
	// changes, since image paths are written relative to them.
	let serializedBlocks = new WeakMap<object, MarkdownBlock>();
	let serializedBlocksFor = '';

	function prosemirrorToMarkdown(doc: any): string {
		const owner = `${$appConfig?.active_vault ?? ''}\n${$activeNotePath}`;
		if (owner !== serializedBlocksFor) {
			serializedBlocks = new WeakMap();
			serializedBlocksFor = owner;
		}
		const blocks: MarkdownBlock[] = [];
		doc.forEach((node: any) => {
			let block = serializedBlocks.get(node);
			if (!block) {
				// Preserve every empty paragraph so markdown round-trip keeps the user's vertical
				// spacing exactly. The parser converts the marker (or a blank line before an image)
				// back to <p></p> on load.
				const isEmpty = node.type.name === 'paragraph' && node.childCount === 0;
				block = { text: isEmpty ? '' : serializeNode(node), isImage: !isEmpty && isImageNode(node), isEmpty };
				serializedBlocks.set(node, block);
			}
			blocks.push(block);
		});
		return joinMarkdownBlocks(blocks);
	}

	function tableToMarkdown(table: any): string {
		const rows: string[][] = [];
		const aligns: (string | null)[] = [];
		let hasHeader = false;
		table.forEach((row: any) => {
			if (row.type.name !== 'tableRow') return;
			const cells: string[] = [];
			row.forEach((cell: any) => {
				if (cell.type.name === 'tableHeader') hasHeader = true;
				if (rows.length === 0) aligns.push(cell.attrs.align || null);
				const cellText: string[] = [];
				cell.forEach((p: any) => {
					cellText.push(serializeInline(p));
				});
				cells.push(cellText.join(' ').replace(/\|/g, '\\|').replace(/\n/g, ' '));
			});
			rows.push(cells);
		});
		if (rows.length === 0) return '';
		const colCount = Math.max(...rows.map(r => r.length));
		const lines: string[] = [];
		rows.forEach((row, i) => {
			lines.push('| ' + row.join(' | ') + ' |');
			if (i === 0 && hasHeader) {
				const delimiters = Array.from({ length: colCount }, (_, column) =>
					aligns[column] === 'left' ? ':---' : aligns[column] === 'center' ? ':---:' : aligns[column] === 'right' ? '---:' : '---');
				lines.push('| ' + delimiters.join(' | ') + ' |');
			}
		});
		return lines.join('\n');
	}

	function resetTableToMarkdown() {
		if (!editor) return;
		const { state } = editor;
		let { tr } = state;
		let changed = false;
		state.doc.descendants((node: any, pos: number) => {
			if (node.type.name === 'tableCell' || node.type.name === 'tableHeader') {
				if (node.attrs.backgroundColor || node.attrs.colwidth || (node.attrs.colspan && node.attrs.colspan > 1) || (node.attrs.rowspan && node.attrs.rowspan > 1)) {
					tr = tr.setNodeMarkup(pos, undefined, {
						...node.attrs,
						backgroundColor: null,
						colwidth: null,
						colspan: 1,
						rowspan: 1,
					});
					changed = true;
				}
			}
			return true;
		});
		if (changed) {
			editor.view.dispatch(tr);
		}
		closeTableContextMenu();
	}

	function serializeHTMLNode(node: ProseMirrorNode): string {
		const container = document.createElement('div');
		container.appendChild(DOMSerializer.fromSchema(editor!.schema).serializeNode(node));
		// The editor shows images through machine-specific asset URLs; notes store portable paths.
		for (const image of container.querySelectorAll('img[src]')) image.setAttribute('src', stripAssetSrc(image.getAttribute('src') ?? ''));
		return container.innerHTML;
	}

	function serializeNode(node: any): string {
		switch (node.type.name) {
			case 'paragraph': {
				const align = node.attrs.textAlign;
				if (align && align !== 'left') {
					return serializeHTMLNode(node) + '\n';
				}
				return serializeParagraphText(node) + '\n';
			}
			case 'heading': {
				const align = node.attrs.textAlign;
				if (align && align !== 'left') {
					return serializeHTMLNode(node) + '\n';
				}
				return '#'.repeat(node.attrs.level) + ' ' + serializeInline(node) + '\n';
			}
			case 'codeBlock':
				return serializeCodeBlock(node.textContent, node.attrs.language || '');
			case 'blockquote': {
				const blocks: string[] = [];
				node.forEach((child: any) => {
					const lines = serializeNode(child).replace(/\n$/, '').split('\n');
					blocks.push(lines.map((l: string) => '> ' + l).join('\n'));
				});
				return blocks.join('\n>\n') + '\n';
			}
			case 'callout':
				return serializeCallout(node, serializeNode);
			case 'bulletList': {
				const items: string[] = [];
				node.forEach((child: any) => items.push('- ' + serializeListItem(child)));
				return items.join('') + '\n';
			}
			case 'orderedList': {
				const items: string[] = [];
				let i = node.attrs.start ?? 1;
				node.forEach((child: any) => { items.push(`${i++}. ` + serializeListItem(child)); });
				return items.join('') + '\n';
			}
			case 'taskList': {
				const items: string[] = [];
				node.forEach((child: any) => {
					const checked = child.attrs.checked ? 'x' : ' ';
					items.push(`- [${checked}] ` + serializeListItem(child));
				});
				return items.join('') + '\n';
			}
			case 'listItem':
			case 'taskItem':
				return serializeListItem(node);
			case 'horizontalRule':
				return '---\n';
			case 'pageBreak':
				return '<div style="page-break-after: always;"></div>\n';
			case 'table': {
				let hasStyling = false;
				const firstRow = node.firstChild;
				let hasHeaderRow = !!firstRow && firstRow.childCount > 0;
				firstRow?.forEach((cell: any) => {
					if (cell.type.name !== 'tableHeader') hasHeaderRow = false;
				});
				node.descendants((child: any) => {
					if (child.type.name === 'tableCell' || child.type.name === 'tableHeader') {
						if (child.attrs.backgroundColor) hasStyling = true;
						if (child.attrs.colwidth) hasStyling = true;
						if (child.attrs.colspan && child.attrs.colspan > 1) hasStyling = true;
						if (child.attrs.rowspan && child.attrs.rowspan > 1) hasStyling = true;
					}
					return true;
				});
				if (hasStyling || !hasHeaderRow) {
					return serializeHTMLNode(node) + '\n';
				}
				return tableToMarkdown(node) + '\n';
			}
			case 'pdfEmbed': {
				const src = node.attrs.src || '';
				const name = node.attrs.name || '';
				return `<div data-pdf-src="${src}" data-pdf-name="${name}" class="pdf-embed"></div>\n`;
			}
			case 'secretBlock': {
				const payload = (node.attrs.payload || '').trim();
				return `\`\`\`helix-secret\n${payload}\n\`\`\`\n`;
			}
			case 'mathBlock': {
				const tex = node.attrs.tex || '';
				return `$$\n${tex}\n$$\n`;
			}
			case 'details': {
				// Markdown HTML blocks end at a blank line, including one inside <pre><code>.
				// Keep the details element on one source line; HTML parsing restores each encoded newline.
				return serializeHTMLNode(node).replace(/\n/g, '&#10;') + '\n';
			}
			case 'image': {
				const src = stripAssetSrc(node.attrs.src || '');
				if (!src) return ''; // Skip images with unresolved blob: URLs
				const alt = node.attrs.alt || '';
				const size = node.attrs['data-size'] || node.attrs.size || 'full';
				const sizeSuffix = size && size !== 'full' ? `|size=${size}` : '';
				return `![${alt}${sizeSuffix}](${src}${serializeImageTitle(node.attrs.title)})\n`;
			}
			case 'imageGrid':
				// Images serialize exactly as they do in a paragraph, one per line.
				return serializeImgGrid({ cols: node.attrs.cols, layout: node.attrs.layout }, serializeInline(node));
			default:
				return node.textContent || '';
		}
	}

	function serializeListItem(node: any): string {
		const blocks: ListItemBlock[] = [];
		node.forEach((child: any, _offset: number, index: number) => {
			const isList = child.type.name === 'bulletList' || child.type.name === 'orderedList' || child.type.name === 'taskList';
			blocks.push({
				isList,
				// Text that starts like a task box in a plain list item must not become a task.
				text: index === 0 && child.type.name === 'paragraph'
					? (node.type.name === 'listItem' ? serializeParagraphText(child).replace(/^\[(?=[ xX]\](?:\s|$))/, '\\[') : serializeParagraphText(child))
					: serializeNode(child),
			});
		});
		return joinListItemBlocks(blocks);
	}

	function inlineMathPayloads(markdown: string): string[] {
		const children = mdit.parseInline(markdown, {})[0]?.children ?? [];
		return children.filter(token => token.type === 'helix_math_inline').map(token => token.content);
	}

	/** The text a line of inline Markdown shows when it is read back, without its formatting. */
	function inlinePlainText(markdown: string): string {
		let text = '';
		for (const token of mdit.parseInline(markdown, {})[0]?.children ?? []) {
			if (token.type === 'text' || token.type === 'code_inline' || token.type === 'helix_wiki') text += token.content;
		}
		return text;
	}

	function readsAsParagraph(markdown: string): boolean {
		const tokens = mdit.parse(markdown, {});
		return tokens.length === 3 && tokens[0].type === 'paragraph_open';
	}

	function serializeInline(node: any): string {
		return serializeInlineMarkdownKeepingDollars(node, stripAssetSrc, inlineMathPayloads, inlinePlainText);
	}

	// Lines of the note as it was last loaded or saved, without indentation; see escapeBlockStarts.
	let sourceLines: { raw: string; lines: Set<string> } | null = null;
	function inNoteSource(line: string): boolean {
		const raw = saves.raw;
		if (sourceLines?.raw !== raw) sourceLines = { raw, lines: new Set(raw.split('\n').map((text) => text.trim())) };
		return sourceLines.lines.has(line.trim());
	}

	/** A paragraph's text, kept from turning into a heading, list, quote or other block on reload. */
	function serializeParagraphText(node: any): string {
		return escapeBlockStarts(serializeInline(node), (markdown) => {
			// Nodes are reused between saves, so an unchanged paragraph is parsed back only once.
			const known = paragraphChecks.get(node);
			if (known?.markdown === markdown) return known.reads;
			const reads = readsAsParagraph(markdown);
			paragraphChecks.set(node, { markdown, reads });
			return reads;
		}, inNoteSource);
	}
	const paragraphChecks = new WeakMap<object, { markdown: string; reads: boolean }>();

	function autofocus(el: HTMLElement) {
		requestAnimationFrame(() => el.focus());
	}

	function closeFromOverlay(event: MouseEvent, close: () => void) {
		if (event.target === event.currentTarget) close();
	}

	function closeOnEscape(event: KeyboardEvent, close: () => void) {
		if (event.key === 'Escape') close();
	}

	function closeFormattingDropdowns() {
		headingDropdown = false;
		colorDropdown = false;
		highlightDropdown = false;
		tablePickerOpen = false;
		alignDropdown = false;
		insertDropdown = false;
	}

	function beginToolbarCustomization() {
		if (isMobile || toolbarSaving) return;
		closeAllDropdowns();
		toolbarBase = JSON.stringify($appConfig?.toolbar_layout ?? null);
		toolbarEditSession++;
		toolbarDraft = resolveToolbarLayout($appConfig?.toolbar_layout, false, !!$appConfig?.ai_provider);
		toolbarSelected = toolbarDraft.main[0] ?? toolbarDraft.more[0] ?? null;
		toolbarSaveError = null;
		toolbarConflict = false;
		moreOpen = true;
		// Mouse entry must not steal the note's selection; keyboard entry stays in the toolbar.
		if (formattingBar?.contains(document.activeElement)) focusToolbarSelection();
	}

	function endToolbarCustomization(restoreFocus = false) {
		toolbarEditSession++;
		cancelToolbarDrag();
		toolbarDraft = null;
		toolbarSelected = null;
		toolbarSaveError = null;
		toolbarConflict = false;
		closeAllDropdowns();
		if (restoreFocus) moreButton?.focus({ preventScroll: true });
	}

	function resetToolbarDraft() {
		if (!toolbarDraft || toolbarSaving) return;
		cancelToolbarDrag();
		toolbarDraft = resolveToolbarLayout(null, false, !!$appConfig?.ai_provider);
		if (!toolbarSelected || !toolbarDraft.main.includes(toolbarSelected)) toolbarSelected = toolbarDraft.main[0] ?? null;
		toolbarSaveError = null;
	}

	async function saveToolbarDraft() {
		if (!toolbarDraft || toolbarSaving || toolbarConflict || !$appConfig) return;
		if (JSON.stringify($appConfig.toolbar_layout ?? null) !== toolbarBase) {
			toolbarConflict = true;
			return;
		}
		cancelToolbarDrag();
		const preference = toolbarLayoutPreference(toolbarDraft);
		const session = toolbarEditSession;
		const base = toolbarBase;
		toolbarPendingPreference = JSON.stringify(preference);
		toolbarSaving = true;
		toolbarSaveError = null;
		try {
			await setToolbarLayout(preference);
			// A concurrent window's event wins over this request's stale local response,
			// including when note navigation already dismissed this customization session.
			const current = JSON.stringify($appConfig?.toolbar_layout ?? null);
			if (!toolbarConflict && (current === base || current === toolbarPendingPreference)) {
				appConfig.update(config => config ? { ...config, toolbar_layout: preference } : config);
			} else if (session === toolbarEditSession) {
				toolbarConflict = true;
			}
			if (session === toolbarEditSession && !toolbarConflict) {
				endToolbarCustomization(!!formattingBar?.contains(document.activeElement));
			}
		} catch (error) {
			if (session === toolbarEditSession) toolbarSaveError = `Could not save toolbar: ${String(error)}`;
		} finally {
			toolbarSaving = false;
			toolbarPendingPreference = null;
		}
	}

	function focusToolbarSelection() {
		const id = toolbarSelected;
		void tick().then(() => {
			if (!toolbarDraft || toolbarSelected !== id || !id) return;
			const button = formattingBar?.querySelector<HTMLButtonElement>(`[data-toolbar-id="${id}"] button`);
			button?.focus({ preventScroll: true });
			button?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
		});
	}

	function moveSelectedToolbarButton(area: ToolbarArea, index: number, focus = false) {
		if (!toolbarDraft || !toolbarSelected || toolbarSaving) return;
		toolbarDraft = moveToolbarButton(toolbarDraft, toolbarSelected, area, index);
		toolbarSaveError = null;
		if (focus) focusToolbarSelection();
	}

	function selectToolbarControl(event: Event) {
		if (!toolbarDraft) return;
		const item = (event.target as Element).closest<HTMLElement>('[data-toolbar-id]');
		if (item && !toolbarSaving) toolbarSelected = item.dataset.toolbarId as ToolbarButtonId;
	}

	function captureToolbarClick(event: MouseEvent) {
		if (!toolbarDraft || !(event.target as Element).closest('[data-toolbar-id]')) return;
		event.preventDefault();
		event.stopPropagation();
		selectToolbarControl(event);
	}

	function customizeToolbarKeydown(event: KeyboardEvent) {
		if (!toolbarDraft || !(event.target as Element).closest('[data-toolbar-id]')) return;
		if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End', 'Enter', ' '].includes(event.key)) return;
		event.preventDefault();
		event.stopPropagation();
		selectToolbarControl(event);
		if (toolbarSaving || !toolbarSelectedArea) return;
		const area = toolbarSelectedArea;
		if (event.key === 'ArrowLeft') moveSelectedToolbarButton(area, toolbarSelectedIndex - 1, true);
		else if (event.key === 'ArrowRight') moveSelectedToolbarButton(area, toolbarSelectedIndex + 2, true);
		else if (event.key === 'Home') moveSelectedToolbarButton(area, 0, true);
		else if (event.key === 'End') moveSelectedToolbarButton(area, toolbarDraft[area].length, true);
		else if (event.key === 'ArrowUp' && area === 'main') moveSelectedToolbarButton('more', toolbarDraft.more.length, true);
		else if (event.key === 'ArrowDown' && area === 'more') moveSelectedToolbarButton('main', toolbarDraft.main.length, true);
	}

	function customizeToolbarItem(node: HTMLElement, options: { id: ToolbarButtonId; selected: boolean }) {
		const button = node.querySelector('button');
		const update = ({ id, selected }: typeof options) => {
			button?.setAttribute('aria-label', toolbarLabels[id]);
			button?.setAttribute('aria-pressed', String(selected));
			button?.setAttribute('aria-describedby', toolbarHintId);
			button?.setAttribute('aria-keyshortcuts', 'ArrowLeft ArrowRight ArrowUp ArrowDown Home End');
		};
		update(options);
		return { update };
	}

	function startToolbarDrag(event: PointerEvent) {
		if (!toolbarDraft || event.button !== 0 || !event.isPrimary) return;
		const item = (event.target as Element).closest<HTMLElement>('[data-toolbar-id]');
		const button = item?.querySelector('button');
		if (!item || !button) return;
		event.preventDefault();
		event.stopPropagation();
		if (toolbarSaving || toolbarDrag) return;
		toolbarSelected = item.dataset.toolbarId as ToolbarButtonId;
		button.focus({ preventScroll: true });
		toolbarDrag = {
			id: toolbarSelected, pointerId: event.pointerId, button, active: false,
			startX: event.clientX, startY: event.clientY, x: event.clientX, y: event.clientY,
		};
		// Capture on the stable bar, never an item that a drop can recreate.
		toolbarCapture = formattingBar ?? null;
		toolbarCapture?.setPointerCapture(event.pointerId);
	}

	function trackToolbarDrag(event: PointerEvent) {
		const drag = toolbarDrag;
		if (!drag || event.pointerId !== drag.pointerId) return;
		drag.x = event.clientX;
		drag.y = event.clientY;
		if (!drag.active && Math.hypot(drag.x - drag.startX, drag.y - drag.startY) < 5) return;
		event.preventDefault();
		if (!drag.active) {
			drag.active = true;
			toolbarDraggedId = drag.id;
			// A detached clone is only a visual ghost; Svelte keeps ownership of every real control.
			const ghost = drag.button.cloneNode(true) as HTMLButtonElement;
			ghost.removeAttribute('id');
			ghost.setAttribute('aria-hidden', 'true');
			ghost.inert = true;
			ghost.style.cssText = 'position:fixed;left:0;top:0;z-index:1001;pointer-events:none;margin:0;width:34px;height:34px;background:var(--bg-primary);color:var(--text-primary);border:1px solid var(--accent);box-shadow:var(--shadow-lg);opacity:.95;';
			document.body.appendChild(ghost);
			toolbarDragGhost = ghost;
			toolbarDragFrame = requestAnimationFrame(updateToolbarDrop);
		}
		if (toolbarDragGhost) toolbarDragGhost.style.transform = `translate(${drag.x + 12}px, ${drag.y + 12}px)`;
	}

	function updateToolbarDrop() {
		toolbarDragFrame = 0;
		const drag = toolbarDrag;
		if (!drag?.active) return;
		const zone = document.elementFromPoint(drag.x, drag.y)?.closest<HTMLElement>('[data-toolbar-area]');
		if (!zone || !formattingBar?.contains(zone)) {
			toolbarDrop = null;
		} else {
			const area = zone.dataset.toolbarArea as ToolbarArea;
			const bounds = zone.getBoundingClientRect();
			// Both regions may wrap and scroll. Continue scrolling while the pointer rests at an edge.
			if (zone.scrollHeight > zone.clientHeight) {
				zone.scrollTop += drag.y < bounds.top + 20 ? -8 : drag.y > bounds.bottom - 20 ? 8 : 0;
			}
			if (zone.scrollWidth > zone.clientWidth) {
				zone.scrollLeft += drag.x < bounds.left + 20 ? -8 : drag.x > bounds.right - 20 ? 8 : 0;
			}
			const items = Array.from(zone.querySelectorAll<HTMLElement>('[data-toolbar-id]'), (node, index) => ({ index, rect: node.getBoundingClientRect() }))
				.filter(({ rect }) => rect.bottom > bounds.top && rect.top < bounds.bottom && rect.right > bounds.left && rect.left < bounds.right);
			if (items.length === 0) {
				toolbarDrop = { area, index: 0, left: bounds.left + 8, top: bounds.top + 8, height: Math.max(24, bounds.height - 16) };
			} else {
				const nearest = items.reduce((best, item) => Math.abs(drag.y - (item.rect.top + item.rect.height / 2)) < Math.abs(drag.y - (best.rect.top + best.rect.height / 2)) ? item : best);
				const row = items.filter(({ rect }) => Math.abs(rect.top - nearest.rect.top) < 3);
				const before = row.find(({ rect }) => drag.x < rect.left + rect.width / 2);
				const edge = before ?? row[row.length - 1];
				toolbarDrop = {
					area, index: edge.index + (before ? 0 : 1),
					left: Math.max(bounds.left + 1, Math.min(before ? edge.rect.left - 2 : edge.rect.right + 1, bounds.right - 3)),
					top: Math.max(bounds.top, edge.rect.top), height: Math.min(edge.rect.bottom, bounds.bottom) - Math.max(edge.rect.top, bounds.top),
				};
			}
		}
		toolbarDragFrame = requestAnimationFrame(updateToolbarDrop);
	}

	function finishToolbarDrag(event: PointerEvent) {
		if (!toolbarDrag || event.pointerId !== toolbarDrag.pointerId) return;
		if (toolbarDrag.active) {
			toolbarDrag.x = event.clientX;
			toolbarDrag.y = event.clientY;
			if (toolbarDragFrame) cancelAnimationFrame(toolbarDragFrame);
			updateToolbarDrop();
		}
		const drop = toolbarDrag.active ? toolbarDrop : null;
		if (drop && toolbarDraft && !toolbarSaving) {
			toolbarDraft = moveToolbarButton(toolbarDraft, toolbarDrag.id, drop.area, drop.index);
			toolbarSaveError = null;
		}
		cancelToolbarDrag();
		if (drop) focusToolbarSelection();
	}

	function cancelToolbarDrag(event?: PointerEvent) {
		if (event && event.pointerId !== toolbarDrag?.pointerId) return;
		const pointerId = toolbarDrag?.pointerId;
		toolbarDrag = null;
		toolbarDraggedId = null;
		toolbarDrop = null;
		if (toolbarDragFrame) cancelAnimationFrame(toolbarDragFrame);
		toolbarDragFrame = 0;
		toolbarDragGhost?.remove();
		toolbarDragGhost = null;
		if (pointerId !== undefined && toolbarCapture?.hasPointerCapture(pointerId)) toolbarCapture.releasePointerCapture(pointerId);
		toolbarCapture = null;
	}

	function closeMore() {
		moreOpen = false;
	}

	function openMore(focus = false) {
		if (!moreOpen) closeFormattingDropdowns();
		moreOpen = true;
		if (focus) void tick().then(() => morePanel?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus({ preventScroll: true }));
	}

	function toggleMore(event: MouseEvent) {
		event.stopPropagation();
		if (toolbarDraft) return;
		if (moreOpen) {
			closeAllDropdowns();
		} else {
			openMore(event.detail === 0);
		}
	}

	function preserveFormattingFocus(event: PointerEvent | MouseEvent) {
		// Keep the DOM selection and mobile IME in the editor. Native buttons still
		// activate on click; keyboard focus and touch scrolling remain available.
		if (event.button === 0 && (event.target as Element).closest('button')) event.preventDefault();
	}

	type FormattingPicker = 'insert' | 'heading' | 'text-color' | 'highlight' | 'alignment' | 'table';
	function toggleFormattingPicker(event: MouseEvent, picker: FormattingPicker) {
		event.stopPropagation();
		const wasOpen = picker === 'insert' ? insertDropdown : picker === 'heading' ? headingDropdown
			: picker === 'text-color' ? colorDropdown : picker === 'highlight' ? highlightDropdown
			: picker === 'alignment' ? alignDropdown : tablePickerOpen;
		closeFormattingDropdowns();
		const clicked = event.currentTarget as HTMLButtonElement;
		formattingTrigger = clicked.closest('.fmt-dropdown')
			? clicked.closest('.fmt-dropdown-wrap')?.querySelector<HTMLButtonElement>(':scope > button') ?? clicked
			: clicked;
		if (wasOpen) return;
		if (picker === 'insert') insertDropdown = true;
		else if (picker === 'heading') headingDropdown = true;
		else if (picker === 'text-color') colorDropdown = true;
		else if (picker === 'highlight') highlightDropdown = true;
		else if (picker === 'alignment') alignDropdown = true;
		else tablePickerOpen = true;
		if (event.detail === 0) void tick().then(() => formattingBar?.querySelector<HTMLButtonElement>('.fmt-dropdown button')?.focus({ preventScroll: true }));
	}

	function handleFormattingBarClick(event: MouseEvent) {
		if (!toolbarDraft && (event.target as Element).closest('button')) closeAllDropdowns();
	}

	function handleFormattingEscape(event: KeyboardEvent) {
		if (event.key !== 'Escape' || event.isComposing || (!moreOpen && !anyDropdownOpen)) return;
		const target = event.target instanceof Element ? event.target : null;
		if (target && !formattingBar?.contains(target)
			&& target.closest('[role="dialog"], [role="menu"], .note-search-bar')) return;
		event.preventDefault();
		event.stopPropagation();
		if (toolbarDraft) {
			if (!toolbarSaving) endToolbarCustomization(!!formattingBar?.contains(document.activeElement));
			return;
		}
		const focusInBar = formattingBar?.contains(document.activeElement);
		if (anyDropdownOpen) {
			const trigger = formattingTrigger;
			closeFormattingDropdowns();
			if (focusInBar) trigger?.focus({ preventScroll: true });
		} else {
			closeMore();
			if (focusInBar) moreButton?.focus({ preventScroll: true });
		}
	}

	function moreKeydown(event: KeyboardEvent) {
		if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
			event.preventDefault();
			openMore(true);
		}
	}

	// Fixed descendants escape the scrolling button strip/list without moving
	// Svelte-owned DOM. Reposition on scrolling, resizing and IME viewport changes.
	function positionFormattingPopup(node: HTMLElement) {
		const trigger = formattingTrigger;
		let frame = 0;
		const position = () => {
			frame = 0;
			if (!trigger || !formattingBar) return;
			const vv = window.visualViewport;
			const leftEdge = (vv?.offsetLeft ?? 0) + 8;
			const topEdge = (vv?.offsetTop ?? 0) + 8;
			const rightEdge = leftEdge + (vv?.width ?? window.innerWidth) - 16;
			const viewportBottom = topEdge + (vv?.height ?? window.innerHeight) - 16;
			const anchor = trigger.isConnected ? trigger.getBoundingClientRect() : formattingBar.getBoundingClientRect();
			const bottom = Math.min(formattingBar.getBoundingClientRect().top - 4, viewportBottom);
			node.style.maxWidth = `${Math.max(0, rightEdge - leftEdge)}px`;
			node.style.maxHeight = `${Math.max(0, bottom - topEdge)}px`;
			node.style.left = `${Math.max(leftEdge, Math.min(anchor.left, rightEdge - node.offsetWidth))}px`;
			node.style.top = `${Math.max(topEdge, bottom - node.offsetHeight)}px`;
		};
		const schedule = () => { if (!frame) frame = requestAnimationFrame(position); };
		position();
		const resize = new ResizeObserver(schedule);
		resize.observe(node);
		if (formattingBar) resize.observe(formattingBar);
		window.addEventListener('resize', schedule);
		document.addEventListener('scroll', schedule, true);
		window.visualViewport?.addEventListener('resize', schedule);
		window.visualViewport?.addEventListener('scroll', schedule);
		return { destroy() {
			if (frame) cancelAnimationFrame(frame);
			resize.disconnect();
			window.removeEventListener('resize', schedule);
			document.removeEventListener('scroll', schedule, true);
			window.visualViewport?.removeEventListener('resize', schedule);
			window.visualViewport?.removeEventListener('scroll', schedule);
		} };
	}

	// ── In-note search functions ──
	let noteSearchTimer: ReturnType<typeof setTimeout> | null = null;
	let noteSearchFrame: number | null = null;
	let noteSearchGeneration = 0;
	let noteSearchAnchor = 0;
	let noteSearchSourceHighlighted = false;
	const searchRevealedDetails = new Set<HTMLElement>();

	function cancelNoteSearchWork() {
		noteSearchGeneration++;
		if (noteSearchTimer !== null) clearTimeout(noteSearchTimer);
		if (noteSearchFrame !== null) cancelAnimationFrame(noteSearchFrame);
		noteSearchTimer = null;
		noteSearchFrame = null;
		noteSearchPending = false;
	}

	function scheduleNoteSearch(reset = false, reveal = false) {
		cancelNoteSearchWork();
		if (reset) source.highlight(null);
		if (reset) {
			noteSearchAnchor = 0;
			noteSearchSourceHighlighted = false;
			noteSearchResults = [];
			noteSearchIndex = 0;
			if (editor && !editor.isDestroyed) {
				editor.view.dispatch(editor.state.tr.setMeta(noteSearchPluginKey, DecorationSet.empty));
			}
		}
		if (!noteSearchOpen || !noteSearchQuery) return;
		noteSearchPending = true;
		const generation = noteSearchGeneration;
		const path = $activeNotePath;
		const isSource = $sourceMode;
		noteSearchTimer = setTimeout(() => {
			noteSearchTimer = null;
			if (generation !== noteSearchGeneration || !noteSearchOpen || path !== $activeNotePath || isSource !== $sourceMode || editorDisposed) return;
			scanNoteSearch();
			// Query changes may reveal a result; live document edits never move the caret
			// or viewport, even if a delayed scan finishes after focus moved elsewhere.
			if (reveal && document.activeElement === noteSearchInput) {
				revealNoteSearchMatch();
			} else if (isSource && noteSearchSourceHighlighted) {
				source.highlight(noteSearchResults[noteSearchIndex] ?? null);
			}
		}, 100);
	}

	function scanNoteSearch() {
		const options = { matchCase: noteSearchMatchCase, wholeWord: noteSearchWholeWord };
		const sourceText = $sourceMode ? source.state.doc.toString() : '';
		noteSearchResults = $sourceMode
			? findSourceMatches(sourceText, noteSearchQuery, options)
			: editor ? findDocumentMatches(editor.state.doc, noteSearchQuery, options) : [];
		noteSearchIndex = closestSearchMatch(noteSearchResults, noteSearchAnchor);
		noteSearchAnchor = noteSearchResults[noteSearchIndex]?.from ?? noteSearchAnchor;
		noteSearchPending = false;
		if (!$sourceMode) applySearchDecorations();
	}

	function scrollToSourceMatch() {
		const match = noteSearchResults[noteSearchIndex];
		if (!match) return;
		noteSearchSourceHighlighted = true;
		source.highlight(match, true);
	}


	// ── Caret carry-over between the rich editor and the markdown source view (issue #125) ──
	// The caret is a ProseMirror doc position in one mode and a character offset in the other,
	// and the markdown<->doc conversion is lossy, so rather than invert the conversion we anchor
	// on VISIBLE TEXT. `scanAlign` walks the markdown source against the doc's plain text (its
	// non-whitespace characters); markup characters don't match the next expected visible char and
	// are skipped, so the scan locks onto real words and self-resynchronises. This stays correct
	// even at the end of a long note and is decoupled from how serialization happens to format.
	//   - stopAtNw >= 0: stop once that many visible chars have matched -> srcOffset is the source
	//     position just past them (maps a doc caret to a source offset).
	//   - limit >= 0: stop at source index `limit` -> nwCount is how many visible chars precede it
	//     (maps a source caret to a doc visible-char count).
	function scanAlign(source: string, docNonWs: string, opts: { limit?: number; stopAtNw?: number }): { srcOffset: number; nwCount: number } {
		const limit = opts.limit ?? -1;
		const stopAtNw = opts.stopAtNw ?? -1;
		if (stopAtNw === 0) return { srcOffset: 0, nwCount: 0 };
		let k = 0;
		let i = 0;
		for (; i < source.length; i++) {
			if (limit >= 0 && i >= limit) break;
			const c = source[i];
			if (c === ' ' || c === '\n' || c === '\t' || c === '\r') continue;
			if (c === docNonWs[k]) {
				k++;
				if (stopAtNw >= 0 && k >= stopAtNw) { i++; break; }
			}
		}
		return { srcOffset: i, nwCount: k };
	}

	// The whole editor doc as plain text with whitespace removed (the alignment alphabet).
	function docNonWhitespace(): string {
		if (!editor) return '';
		return editor.state.doc.textBetween(0, editor.state.doc.content.size, '\n', '').replace(/\s/g, '');
	}

	// Editor doc position just after the `targetNw`-th non-whitespace character.
	function docPosForNonWsCount(doc: any, targetNw: number): number {
		if (targetNw <= 0) return 0;
		const nwAt = (pos: number) => doc.textBetween(0, pos, '\n', '').replace(/\s/g, '').length;
		let lo = 0;
		let hi = doc.content.size;
		while (lo < hi) {
			const mid = (lo + hi) >> 1;
			if (nwAt(mid) >= targetNw) hi = mid; else lo = mid + 1;
		}
		return lo;
	}


	function applySearchDecorations() {
		if (!editor) return;
		// Count/navigate every hit, but bound passive DOM highlights in dense notes.
		const limit = 500;
		const start = Math.max(0, Math.min(noteSearchIndex - limit / 2, noteSearchResults.length - limit));
		const end = Math.min(noteSearchResults.length, start + limit);
		const decorations: Decoration[] = [];
		for (let i = start; i < end; i++) {
			const match = noteSearchResults[i];
			decorations.push(Decoration.inline(match.from, match.to, {
				class: i === noteSearchIndex ? 'note-search-match note-search-active' : 'note-search-match',
			}));
		}
		const decoSet = DecorationSet.create(editor.state.doc, decorations);
		const tr = editor.state.tr.setMeta(noteSearchPluginKey, decoSet);
		editor.view.dispatch(tr);
	}

	function revealNoteSearchMatch() {
		if ($sourceMode) {
			scrollToSourceMatch();
			return;
		}
		if (!editor || !noteSearchResults.length) return;
		if (noteSearchFrame !== null) cancelAnimationFrame(noteSearchFrame);
		const generation = noteSearchGeneration;
		noteSearchFrame = requestAnimationFrame(() => {
			noteSearchFrame = null;
			if (!noteSearchOpen || generation !== noteSearchGeneration || $sourceMode) return;
			const el = editor?.view.dom.querySelector<HTMLElement>('.note-search-active');
			if (!el) return;
			for (let parent = el.parentElement; parent; parent = parent.parentElement) {
				if (parent.matches('[data-type="details"]') && !parent.classList.contains('is-open')) {
					searchRevealedDetails.add(parent);
					openDetailsEl(parent);
				}
			}
			el.scrollIntoView({ block: 'center', behavior: 'instant' });
		});
	}

	function navigateNoteSearch(direction: number) {
		if (!noteSearchOpen) return;
		const pending = noteSearchPending;
		cancelNoteSearchWork();
		if (pending) scanNoteSearch();
		if (noteSearchResults.length) {
			noteSearchIndex = (noteSearchIndex + direction + noteSearchResults.length) % noteSearchResults.length;
			noteSearchAnchor = noteSearchResults[noteSearchIndex].from;
			if (!$sourceMode) applySearchDecorations();
			revealNoteSearchMatch();
		}
		noteSearchInput?.focus({ preventScroll: true });
	}

	function noteSearchNext() { navigateNoteSearch(1); }
	function noteSearchPrev() { navigateNoteSearch(-1); }

	export function openNoteSearch(replace = false) {
		++targetRevealRequest;
		clearTargetReveal();
		// Asking for replace while find is already showing moves on to the replace field.
		const toReplace = replace && noteSearchOpen && !!noteSearchQuery;
		noteSearchOpen = true;
		if (replace) noteReplaceOpen = true;
		tick().then(() => {
			if (!noteSearchOpen || editorDisposed) return;
			if (toReplace) noteReplaceInput?.focus({ preventScroll: true });
			else noteSearchInput?.focus({ preventScroll: true });
		});
	}

	function canReplaceInNote(): boolean {
		if ($readOnly || $viewerNote || trashPreview || showHistory || externalOperationBusy) return false;
		return $sourceMode ? !!source.view : !!editor?.isEditable;
	}

	// Replacing text shown by a wiki link changes what the link says, not where it points:
	// [[Home]] becomes [[Home|House]]. Otherwise it would count as retitling the linked note.
	function keepWikiLinkTarget(tr: any, from: number) {
		const wikiLink = tr.doc.type.schema.marks.wikiLink;
		if (!wikiLink) return;
		const resolved = tr.doc.resolve(from);
		const parent = resolved.parent;
		const index = resolved.index();
		const mark = parent.maybeChild(index)?.marks.find((m: any) => m.type === wikiLink);
		if (!mark || mark.attrs.aliased) return;
		let first = index;
		let last = index;
		const linked = (i: number) => !!parent.maybeChild(i)?.marks.some((m: any) => m.eq(mark));
		while (first > 0 && linked(first - 1)) first--;
		while (linked(last + 1)) last++;
		let start = resolved.start();
		for (let i = 0; i < first; i++) start += parent.child(i).nodeSize;
		let end = start;
		let shown = '';
		for (let i = first; i <= last; i++) { end += parent.child(i).nodeSize; shown += parent.child(i).text ?? ''; }
		// The mark does not exclude itself, so the old one has to go first.
		tr.removeMark(start, end, mark);
		tr.addMark(start, end, wikiLink.create({ ...mark.attrs, title: mark.attrs.title || shown, aliased: true }));
	}

	function replaceNoteMatches(matches: NoteSearchMatch[]) {
		const insert = noteReplaceValue;
		if ($sourceMode) {
			source.dispatch({ changes: matches.map(({ from, to }) => ({ from, to, insert })), userEvent: 'input.replace' });
			return;
		}
		if (!editor) return;
		const tr = editor.state.tr;
		// Last to first, so earlier positions stay valid. One transaction is one undo step.
		for (let i = matches.length - 1; i >= 0; i--) {
			keepWikiLinkTarget(tr, matches[i].from);
			tr.insertText(insert, matches[i].from, matches[i].to);
		}
		editor.view.dispatch(tr);
	}

	function replaceNoteMatch() {
		if (!noteSearchOpen || !canReplaceInNote()) return;
		if (noteSearchPending) { cancelNoteSearchWork(); scanNoteSearch(); }
		const match = noteSearchResults[noteSearchIndex];
		if (!match) return;
		replaceNoteMatches([match]);
		// Continue behind the inserted text, so a replacement that contains the query is not found again.
		cancelNoteSearchWork();
		noteSearchAnchor = match.from + noteReplaceValue.length;
		scanNoteSearch();
		if (noteSearchResults.length && noteSearchResults[noteSearchIndex].from < noteSearchAnchor) {
			noteSearchIndex = 0;
			noteSearchAnchor = noteSearchResults[0].from;
			if (!$sourceMode) applySearchDecorations();
		}
		if (noteSearchResults.length) revealNoteSearchMatch();
		else source.highlight(null);
	}

	function replaceAllNoteMatches() {
		if (!noteSearchOpen || !canReplaceInNote()) return;
		if (noteSearchPending) { cancelNoteSearchWork(); scanNoteSearch(); }
		const matches = replaceableMatches(noteSearchResults);
		if (!matches.length) return;
		replaceNoteMatches(matches);
		cancelNoteSearchWork();
		scanNoteSearch();
		source.highlight(null);
	}

	function closeNoteSearch(restoreFocus = true) {
		cancelNoteSearchWork();
		noteSearchOpen = false;
		noteSearchQuery = '';
		noteReplaceOpen = false;
		noteReplaceValue = '';
		noteSearchResults = [];
		noteSearchIndex = 0;
		noteSearchSourceHighlighted = false;
		source.highlight(null);
		for (const element of searchRevealedDetails) {
			if (!element.isConnected || !editor || !editor.view.dom.contains(element)) continue;
			const content = element.querySelector<HTMLElement>('[data-type="detailsContent"]');
			if (!content) continue;
			const resolved = editor.state.doc.resolve(editor.view.posAtDOM(content, 0));
			let persistedOpen = false;
			for (let depth = resolved.depth; depth > 0; depth--) {
				if (resolved.node(depth).type.name === 'details') {
					persistedOpen = !!resolved.node(depth).attrs.open;
					break;
				}
			}
			if (!persistedOpen && element.classList.contains('is-open')) {
				element.classList.remove('is-open');
				content.dispatchEvent(new Event('toggleDetailsContent'));
			}
		}
		searchRevealedDetails.clear();
		if (editor && !editor.isDestroyed) {
			editor.view.dispatch(editor.state.tr.setMeta(noteSearchPluginKey, DecorationSet.empty));
		}
		if (restoreFocus) {
			if ($sourceMode) source.focus();
			else editor?.commands.focus(undefined, { scrollIntoView: false });
		}
	}

	function handleNoteSearchKey(event: KeyboardEvent) {
		if (!noteSearchOpen || event.isComposing || event.key !== 'F3' || event.ctrlKey || event.metaKey || event.altKey) return;
		event.preventDefault();
		event.shiftKey ? noteSearchPrev() : noteSearchNext();
	}

	function stripAssetSrc(src: string): string {
		// Keep pending images in the captured body until the save queue persists them.
		return src.startsWith('blob:') ? src : assetSourceToMarkdown(src, $activeNotePath, $appConfig?.active_vault ?? null);
	}

	function htmlToMarkdown(html: string): string {
		let md = html;
		// Code blocks MUST be converted before inline code to avoid corruption
		md = md.replace(/<pre><code[^>]*class="language-(\w+)"[^>]*>([\s\S]*?)<\/code><\/pre>/gi, (_, lang, code) => {
			const stripped = code.replace(/<[^>]+>/g, '');
			const decoded = stripped.replace(/&amp;/g, '&').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&#39;/g, "'");
			return '```' + lang + '\n' + decoded + '\n```\n';
		});
		md = md.replace(/<pre><code[^>]*>([\s\S]*?)<\/code><\/pre>/gi, (_, code) => {
			const stripped = code.replace(/<[^>]+>/g, '');
			const decoded = stripped.replace(/&amp;/g, '&').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&#39;/g, "'");
			return '```\n' + decoded + '\n```\n';
		});
		md = md.replace(/<h1[^>]*>(.*?)<\/h1>/gi, '# $1\n');
		md = md.replace(/<h2[^>]*>(.*?)<\/h2>/gi, '## $1\n');
		md = md.replace(/<h3[^>]*>(.*?)<\/h3>/gi, '### $1\n');
		md = md.replace(/<h4[^>]*>(.*?)<\/h4>/gi, '#### $1\n');
		md = md.replace(/<h5[^>]*>(.*?)<\/h5>/gi, '##### $1\n');
		md = md.replace(/<h6[^>]*>(.*?)<\/h6>/gi, '###### $1\n');
		md = md.replace(/<strong>(.*?)<\/strong>/gi, '**$1**');
		md = md.replace(/<b>(.*?)<\/b>/gi, '**$1**');
		md = md.replace(/<em>(.*?)<\/em>/gi, '*$1*');
		md = md.replace(/<i>(.*?)<\/i>/gi, '*$1*');
		md = md.replace(/<s>(.*?)<\/s>/gi, '~~$1~~');
		md = md.replace(/<del>(.*?)<\/del>/gi, '~~$1~~');
		md = md.replace(/<u>(.*?)<\/u>/gi, '<u>$1</u>');
		md = md.replace(/<sub>(.*?)<\/sub>/gi, '~$1~');
		md = md.replace(/<sup>(.*?)<\/sup>/gi, '^$1^');
		md = md.replace(/<code>(.*?)<\/code>/gi, '`$1`');
		md = md.replace(/<mark data-color="([^"]*)">(.*?)<\/mark>/gi, '<mark data-color="$1">$2</mark>');
		md = md.replace(/<mark>(.*?)<\/mark>/gi, '==$1==');
		md = md.replace(/<blockquote[^>]*>([\s\S]*?)<\/blockquote>/gi, (_, content) => {
			return content
				.replace(/<p[^>]*>(.*?)<\/p>/gi, '> $1\n')
				.replace(/<br\s*\/?>/gi, '\n> ');
		});
		md = md.replace(/<a[^>]*href="([^"]*)"[^>]*>(.*?)<\/a>/gi, (_m, href, text) => {
			// Decode percent-encoded href back to readable form for markdown source
			// Spaces are re-encoded by markdownToHtml preprocessing before markdown-it parsing
			const decoded = decodeURIComponent(href);
			return `[${text}](${decoded})`;
		});
		md = md.replace(/<img[^>]*>/gi, (match) => {
			const srcMatch = match.match(/src="([^"]*)"/);
			const altMatch = match.match(/alt="([^"]*)"/);
			const sizeMatch = match.match(/data-size="([^"]*)"/);
			const src = srcMatch ? stripAssetSrc(srcMatch[1]) : '';
			const alt = altMatch ? altMatch[1] : '';
			const size = sizeMatch ? sizeMatch[1] : 'full';
			const sizeSuffix = size && size !== 'full' ? `|size=${size}` : '';
			return `![${alt}${sizeSuffix}](${src})`;
		});
		// Preserve PDF embeds as raw HTML
		const pdfs: string[] = [];
		md = md.replace(/<div[^>]*data-pdf-src="([^"]*)"[^>]*>[\s\S]*?<\/div>/gi, (match, src) => {
			// Store with the relative path - we strip convertFileSrc URLs on save
			const nameMatch = match.match(/data-pdf-name="([^"]*)"/);
			const name = nameMatch ? nameMatch[1] : src.split('/').pop() || 'file.pdf';
			pdfs.push(`<div data-pdf-src="${src}" data-pdf-name="${name}" class="pdf-embed"></div>`);
			return `\n%%PDF_${pdfs.length - 1}%%\n`;
		});
		// Preserve details/accordion blocks as raw HTML
		const detailsBlocks: string[] = [];
		md = md.replace(/<details[\s\S]*?<\/details>/gi, (match) => {
			detailsBlocks.push(match);
			return `\n%%DETAILS_${detailsBlocks.length - 1}%%\n`;
		});
		// Preserve tables as raw HTML (markdown tables are too limited)
		const tables: string[] = [];
		md = md.replace(/<table[\s\S]*?<\/table>/gi, (match) => {
			tables.push(match);
			return `\n%%TABLE_${tables.length - 1}%%\n`;
		});

		md = md.replace(/<hr\s*\/?>/gi, '---\n');
		md = md.replace(/<ul[^>]*>([\s\S]*?)<\/ul>/gi, (_, content) => {
			return content.replace(/<li[^>]*><p[^>]*>(.*?)<\/p><\/li>/gi, '- $1\n')
				.replace(/<li[^>]*>(.*?)<\/li>/gi, '- $1\n');
		});
		md = md.replace(/<ol[^>]*>([\s\S]*?)<\/ol>/gi, (_, content) => {
			let i = 0;
			return content.replace(/<li[^>]*><p[^>]*>(.*?)<\/p><\/li>/gi, () => `${++i}. $1\n`)
				.replace(/<li[^>]*>(.*?)<\/li>/gi, () => `${++i}. $1\n`);
		});
		md = md.replace(/<li[^>]*data-checked="true"[^>]*>(.*?)<\/li>/gi, '- [x] $1\n');
		md = md.replace(/<li[^>]*data-checked="false"[^>]*>(.*?)<\/li>/gi, '- [ ] $1\n');
		md = md.replace(/<p[^>]*>(.*?)<\/p>/gi, '$1\n\n');
		md = md.replace(/<br\s*\/?>/gi, '\n');
		md = md.replace(/<[^>]+>/g, '');
		md = md.replace(/&amp;/g, '&');
		md = md.replace(/&lt;/g, '<');
		md = md.replace(/&gt;/g, '>');
		md = md.replace(/&quot;/g, '"');
		md = md.replace(/&#39;/g, "'");
		md = md.replace(/\n{3,}/g, '\n\n');
		// Restore table HTML
		tables.forEach((table, i) => {
			md = md.replace(`%%TABLE_${i}%%`, '\n' + table + '\n');
		});
		// Restore PDF embeds
		pdfs.forEach((pdf, i) => {
			md = md.replace(`%%PDF_${i}%%`, '\n' + pdf + '\n');
		});
		// Restore details/accordion blocks
		detailsBlocks.forEach((block, i) => {
			md = md.replace(`%%DETAILS_${i}%%`, '\n' + block + '\n');
		});
		return md.trim() + '\n';
	}


	function markdownToHtml(md: string, snapshot = false): string {
		const src = snapshot ? md : stripTitleH1(md);
		// Render once; parser rules keep code and math payloads out of prose transformations.
		let html = mdit.render(src, {
			wikiLinkTitles: $appConfig?.enable_wiki_links ? wikiLinkTitlesCache : undefined,
		});

		// PDF embeds are raw HTML here; literal code is already escaped by the parser.
		html = html.replace(/<div[^>]*data-pdf-src="([^"]*)"[^>]*data-pdf-name="([^"]*)"[^>]*>[^<]*<\/div>/gi, (_, pdfSrc, name) => {
			const root = currentNoteAssetRoot();
			const local = resolveFileReference(pdfSrc, root ? `${root}/.vault-root` : $activeNotePath, root);
			const showInline = !snapshot && !isMobile && ($appConfig?.pdf_preview ?? false);
			const safeSrc = mdit.utils.escapeHtml(pdfSrc);
			const safeName = mdit.utils.escapeHtml(name);
			if (showInline) {
				const pdfHeight = $appConfig?.pdf_height ?? 600;
				const displaySrc = convertFileSrc(local.path) + local.suffix;
				return `<div data-pdf-src="${safeSrc}" data-pdf-name="${safeName}" class="pdf-embed"><iframe src="${mdit.utils.escapeHtml(displaySrc)}" width="100%" height="${pdfHeight}px"></iframe><p class="pdf-label">${safeName}</p></div>`;
			}
			return `<div data-pdf-src="${safeSrc}" data-pdf-name="${safeName}" class="pdf-embed-mobile"><a href="${mdit.utils.escapeHtml(pdfSrc)}" class="pdf-link-mobile">\uD83D\uDCC4 ${safeName}</a></div>`;
		});

		html = normalizeImageHtml(html);
		if (snapshot) {
			html = DOMPurify.sanitize(html, {
				USE_PROFILES: { html: true },
				ADD_URI_SAFE_ATTR: ['src'],
				FORBID_TAGS: ['style', 'iframe', 'object', 'embed', 'form', 'input', 'button', 'video', 'audio', 'source', 'svg', 'math', 'link'],
			});
		}

		// Post-process: turn Obsidian callout blockquotes (> [!type] ...) into callout blocks.
		if (html.includes('[!')) {
			const template = document.createElement('template');
			template.innerHTML = html;
			for (const child of Array.from(template.content.children)) transformCalloutBlockquotes(child);
			html = template.innerHTML;
		}

		return html;
	}

	function escapeHtml(str: string): string {
		return str.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
	}

	// When editorElement appears in DOM, initialize TipTap.
	// On mobile, pre-create editor with empty content so first note load is fast.
	$effect(() => {
		if (editorElement && !editor) {
			if (pendingContent !== null) {
				createEditor(pendingContent);
				pendingContent = null;
				updateCounts();
				if (showOutline) updateOutline();
			} else if (isMobile && !lastSourceMode) {
				createEditor('');
			}
		}
	});

	$effect(() => {
		if (!anyDropdownOpen && !moreOpen) return;
		function onAway(event: Event) {
			if (!toolbarDraft && !formattingBar?.contains(event.target as Node)) closeAllDropdowns();
		}
		document.addEventListener('pointerdown', onAway);
		document.addEventListener('focusin', onAway);
		window.addEventListener('keydown', handleFormattingEscape, true);
		return () => {
			document.removeEventListener('pointerdown', onAway);
			document.removeEventListener('focusin', onAway);
			window.removeEventListener('keydown', handleFormattingEscape, true);
		};
	});

	$effect(() => {
		void $activeNotePath;
		void $sourceMode;
		void $viewerNote;
		void $readOnly;
		untrack(() => endToolbarCustomization());
	});

	$effect(() => {
		const preference = JSON.stringify($appConfig?.toolbar_layout ?? null);
		untrack(() => {
			if (toolbarDraft && preference !== toolbarBase && preference !== toolbarPendingPreference) toolbarConflict = true;
		});
	});

	$effect(() => {
		if (!toolbarDraft) return;
		const blur = () => cancelToolbarDrag();
		window.addEventListener('pointermove', trackToolbarDrag, { passive: false });
		window.addEventListener('pointerup', finishToolbarDrag);
		window.addEventListener('pointercancel', cancelToolbarDrag);
		window.addEventListener('lostpointercapture', cancelToolbarDrag);
		window.addEventListener('blur', blur);
		return () => {
			window.removeEventListener('pointermove', trackToolbarDrag);
			window.removeEventListener('pointerup', finishToolbarDrag);
			window.removeEventListener('pointercancel', cancelToolbarDrag);
			window.removeEventListener('lostpointercapture', cancelToolbarDrag);
			window.removeEventListener('blur', blur);
			cancelToolbarDrag();
		};
	});

	// Count immediately when Info opens or the document/mode changes, but never
	// serialize source here: ordinary typing only schedules the debounced pass.
	$effect(() => {
		if (showInfo) {
			void $sourceMode;
			void $activeNotePath;
			void editorReady;
			untrack(updateCounts);
		}
		return cancelCounts;
	});


	// Close in-note search when switching notes
	let prevSearchPath = '';
	$effect(() => {
		const path = $activeNotePath ?? '';
		if (prevSearchPath && path !== prevSearchPath) {
			untrack(() => closeNoteSearch(false));
		}
		prevSearchPath = path;
	});

	let previousSearchSettings = '';
	$effect(() => {
		const settings = JSON.stringify([noteSearchOpen, noteSearchQuery, noteSearchMatchCase, noteSearchWholeWord, $sourceMode, editorReady]);
		const reset = settings !== previousSearchSettings;
		previousSearchSettings = settings;
		untrack(() => scheduleNoteSearch(reset, reset));
		return cancelNoteSearchWork;
	});


	// React to activeNotePath changes from external sources (e.g. search panel)
	$effect(() => {
		const path = $activeNotePath;
		const note = $activeNote;
		if (!path) {
			// Note was deselected (e.g. deleted) - destroy editor so it reinits on next note
			cancelCounts();
			closeAiMenu();
			closeNoteSearch(false);
			destroyEditor();
			loadedPath = '';
			return;
		}
		if (note && path !== loadedPath) {
			loadNote(path, note.content);
		}
	});

	function destroyEditor() {
		if (!$sourceMode) clearOutline();
		saves.cancelScheduled();
		cancelProgressiveLoad();
		if (editor) {
			editor.destroy();
			editor = null;
		}
		mathObserver?.disconnect();
		mathObserver = null;
		editorReady = false;
		closeSlashMenu();
	}

	function createEditor(content: string) {
		if (!editorElement) return;
		cancelProgressiveLoad();
		if (editor) {
			editor.destroy();
			editor = null;
		}
		mathObserver?.disconnect();
		mathObserver = null;

		isLargeDoc = content.length > LARGE_DOC_CHARS;
		const html = markdownToHtml(content);

		editor = new Editor({
			element: editorElement,
			editable: !$readOnly,
			extensions: [
				MixedListShortcuts,
				ProgressiveLoadGuard,
				EditorStarterKit.configure({ code: { HTMLAttributes: { style: 'white-space: pre-wrap' } }, codeBlock: false, link: false, underline: false }),
				FocusedPlaceholder,
				TaskList,
				TaskItem.extend(keepCodeWhitespace).configure({ nested: true }),
				Table.configure({ resizable: true }),
				TableRow,
				CustomTableCell,
				CustomTableHeader,
				Link.configure({ openOnClick: false, HTMLAttributes: { class: 'editor-link' }, isAllowedUri: (url, ctx) => ctx.defaultValidate(url) || !url.startsWith('javascript:'), shouldAutoLink: (url) => /^https?:\/\//.test(url) }),
				CustomImage.configure({ inline: true, HTMLAttributes: { class: 'editor-image', crossorigin: 'anonymous' } }),
				Highlight.configure({ multicolor: true }),
				Typography,
				Underline,
				Subscript,
				Superscript,
				TextStyle,
				Color,
				CachedCodeBlockLowlight.configure({ lowlight, enableTabIndentation: true, defaultLanguage: 'text' }),
				CodeBlockInputScroll,
				CodeBlockLanguageSelect,
				CopyButtonExtension,
				MermaidRenderer,
				PdfEmbed,
				SecretBlock,
				MathBlock,
				MathInline,
				PageBreak,
				Callout,
				CalloutTyping,
				ImageGrid.configure({ onUngroup: ungroupImageGridAt, onMoveImage: moveGridImageByIndex }),
				DetailsWithBodyBackspace.configure({ persist: true, HTMLAttributes: { class: 'editor-details' } }),
				DetailsSummary,
				DetailsContent,
				Extension.create({
					name: 'collapsibleKeymap',
					addProseMirrorPlugins() {
						return [new Plugin({
							key: new PluginKey('collapsibleKeymap'),
							props: {
								handleDOMEvents: {
									keydown(view, event) {
										const isInsertDetails = (event.ctrlKey || event.metaKey)
											&& !event.shiftKey
											&& !event.altKey
											&& event.code === 'Period';
										if (isInsertDetails) {
											if (get(readOnly)) return false;
											event.preventDefault();
											insertDetails();
											return true;
										}
										const isTab = event.key === 'Tab' && !event.shiftKey && !event.altKey && !event.ctrlKey && !event.metaKey;
										const isEnter = event.key === 'Enter' && !event.shiftKey && !event.altKey && !event.ctrlKey && !event.metaKey;
										if (!isTab && !isEnter) return false;
										const { schema, selection } = view.state;
										const from = selection.$from;
										let summaryDepth = -1;
										for (let d = from.depth; d >= 0; d--) {
											if (from.node(d).type === schema.nodes.detailsSummary) { summaryDepth = d; break; }
										}
										if (summaryDepth === -1) return false;
										event.preventDefault();
										const detailsDepth = summaryDepth - 1;
										const detailsNode = from.node(detailsDepth);
										let detailsContentPos: number | null = null;
										let pos = from.start(detailsDepth);
										for (let i = 0; i < detailsNode.childCount; i++) {
											const child = detailsNode.child(i);
											if (child.type === schema.nodes.detailsContent) { detailsContentPos = pos + 1; break; }
											pos += child.nodeSize;
										}
										if (detailsContentPos === null) return true;
										// Open the section if it is closed
										const domPos = view.domAtPos(from.pos);
										let domNode = domPos.node as HTMLElement;
										if (domNode.nodeType === 3) domNode = domNode.parentElement as HTMLElement;
										const detailsEl = domNode?.closest('[data-type="details"]') as HTMLElement | null;
										if (detailsEl) openDetailsEl(detailsEl);
										// Sync open state into document + move cursor (single transaction)
										const detailsPos = from.before(detailsDepth);
										const tr = view.state.tr.setNodeMarkup(detailsPos, undefined, { open: true });
										const mappedContentPos = tr.mapping.map(detailsContentPos);
										tr.setSelection(Selection.near(tr.doc.resolve(mappedContentPos), 1));
										view.dispatch(tr.scrollIntoView());
										view.focus();
										return true;
									}
								}
							}
						})];
					}
				}),
				Extension.create({
					name: 'detailsOpenAttrSync',
					addProseMirrorPlugins() {
						return [new Plugin({
							key: new PluginKey('detailsOpenAttrSync'),
							props: {
								handleDOMEvents: {
									click(view, event) {
										const btn = (event.target as HTMLElement)?.closest?.('button');
										const detailsEl = btn && btn.parentElement?.getAttribute('data-type') === 'details'
											? (btn.parentElement as HTMLElement) : null;
										if (!detailsEl) return false;
										// The extension's button handler already toggled the is-open class; sync
										// node.attrs.open to it. Fixes the first node (pos 0), where upstream's
										// `if (!pos)` guard skips persisting the attribute.
										const isOpen = detailsEl.classList.contains('is-open');
										const probe = detailsEl.querySelector('[data-type="detailsContent"]') ?? detailsEl;
										let pos: number;
										try { pos = view.posAtDOM(probe, 0); } catch { return false; }
										const resolved = view.state.doc.resolve(pos);
										for (let d = resolved.depth; d >= 0; d--) {
											if (resolved.node(d).type.name === 'details') {
												if (resolved.node(d).attrs.open !== isOpen) {
													view.dispatch(view.state.tr.setNodeMarkup(resolved.before(d), undefined, { open: isOpen }));
												}
												break;
											}
										}
										return false;
									}
								}
							}
						})];
					}
				}),
				TextAlign.configure({ types: ['heading', 'paragraph'] }).extend({
					addKeyboardShortcuts: () => ({}),
				}),
				CtrlEndScrollPastEnd,
				HeadingShortcuts,
				StrikeShortcut,
				WrapSelectedText,
				SlashCommands,
				TaskMetaMenu,
				MoveLineShortcuts,
					TabIndent,
					NoteSearchExtension,
					TargetRevealExtension,
					ColorSwatch,
				TaskMetaDim,
				...($appConfig?.enable_wiki_links ? [WikiLink, WikiLinkAutocomplete] : []),
			],
			content: html,
			editorProps: {
				attributes: { class: 'editor-content', spellcheck: 'false' },
				handleDOMEvents: {
					// Prevent focus-caused scroll jumps when clicking details toggle buttons.
					// Pre-focusing with preventScroll means TipTap's focus() call sees
					// hasFocus()=true and skips its scrolling view.focus() call.
					// For task checkboxes: lock scroll on mousedown (before any dispatch fires)
					// so that any synchronous or async scroll caused by the toggle is reverted.
					mousedown: (view, event) => {
						const target = event.target as HTMLElement;
						if (target.closest('[data-type="details"] > button')) {
							event.preventDefault();
							if (!view.hasFocus()) {
								(view.dom as HTMLElement).focus({ preventScroll: true });
							}
						}
						if (target.closest('li[data-checked] > label')) {
							const editorBody = target.closest('.editor-body') as HTMLElement | null;
							if (editorBody) {
								const savedScroll = editorBody.scrollTop;
								const restore = () => { editorBody!.scrollTop = savedScroll; };
								editorBody.addEventListener('scroll', restore);
								// Remove after 200ms - covers synchronous, rAF, and setTimeout-based scrolls
								setTimeout(() => editorBody!.removeEventListener('scroll', restore), 200);
							}
						}
					},
					// Let task-list checkboxes be ticked in View Mode (read-only) without
					// entering edit mode - e.g. a shopping list on mobile, where editing would
					// pop the soft keyboard. TipTap leaves the checkbox clickable in read-only
					// but reverts the toggle on `change`; we intercept the click, cancel the
					// native toggle, and dispatch the attr change ourselves so it persists via
					// the normal onUpdate -> autoSave path. Edit mode keeps TipTap's behaviour.
					click: (view, event) => {
						if (!get(readOnly)) return false;
						if (get(viewerNote)) return false; // external viewer files are never saved
						const target = event.target as HTMLElement;
						const label = target.closest('li[data-checked] > label');
						if (!label) return false;
						// Cancel the native checkbox toggle (and the label->input click cascade)
						// so it doesn't fight our transaction; the node-view update will set the
						// visual state from the new doc value.
						event.preventDefault();
						const li = label.closest('li') as HTMLElement | null;
						if (!li) return false;
						// Probe position from the content <div> (the real contentDOM), not the <li>
						// outer node-view element, so posAtDOM lands inside the task item's content.
						const probe = (li.querySelector(':scope > div') as HTMLElement | null) || li;
						const pos = view.posAtDOM(probe, 0);
						const resolved = view.state.doc.resolve(pos);
						let itemPos = -1;
						let attrs: Record<string, any> | null = null;
						for (let d = resolved.depth; d >= 0; d--) {
							if (resolved.node(d).type.name === 'taskItem') {
								itemPos = resolved.before(d);
								attrs = resolved.node(d).attrs;
								break;
							}
						}
						if (itemPos < 0) {
							// Fallback: pos landed just before the item rather than inside it.
							const n = view.state.doc.nodeAt(pos);
							if (n && n.type.name === 'taskItem') { itemPos = pos; attrs = n.attrs; }
						}
						if (itemPos < 0 || !attrs) return false;
						view.dispatch(view.state.tr.setNodeMarkup(itemPos, undefined, { ...attrs, checked: !attrs.checked }));
						return true;
					},
					// Prevent native text drag - it causes copy-instead-of-move in Tauri's webview.
					// File drops from OS are handled by Tauri's onDragDropEvent listener instead.
					dragstart: (_view, event) => {
						const dt = event.dataTransfer;
						if (!dt || dt.files.length === 0) {
							event.preventDefault();
						}
					},
				},
				handleDrop: (_view, event) => handleFileDrop(event),
				handlePaste: (_view, event) => handleFilePaste(event),
				// Strip color / font styling from pasted HTML so the editor uses its own theme.
				// Keeps semantic marks (bold, italic, links, headings, alignment) - drops only the
				// inline visual styles that fight the app's theme (color, bg-color, font-family, font-size).
				transformPastedHTML: (html: string) => {
					html = normalizeImageHtml(html);
					if (!/style=|<font/i.test(html)) return html;
					try {
						const doc = new DOMParser().parseFromString(html, 'text/html');
						doc.querySelectorAll('[style]').forEach((el) => {
							const style = (el as HTMLElement).style;
							style.color = '';
							style.backgroundColor = '';
							style.fontFamily = '';
							style.fontSize = '';
							if (!style.cssText.trim()) el.removeAttribute('style');
						});
						doc.querySelectorAll('font').forEach((el) => {
							el.removeAttribute('color');
							el.removeAttribute('face');
							el.removeAttribute('size');
						});
						return doc.body.innerHTML;
					} catch (e) {
						console.warn('[paste] style strip failed', e);
						return html;
					}
				},
			},
			onTransaction: ({ transaction, appendedTransactions }) => {
				if (imageToolbar && (transaction.docChanged || appendedTransactions.some((tr) => tr.docChanged))) closeStaleImageToolbar();
				if (showOutline && !$sourceMode && outlineOrigin && isOriginCurrent(outlineOrigin)
					&& (transaction.docChanged || appendedTransactions.some(tr => tr.docChanged))) {
					outlineHeadings = mapDocumentHeadings(outlineHeadings, transaction.mapping);
					for (const tr of appendedTransactions) outlineHeadings = mapDocumentHeadings(outlineHeadings, tr.mapping);
					scheduleOutline();
					scheduleOutlineActive();
				}
				if (noteSearchOpen && !$sourceMode && (transaction.docChanged || appendedTransactions.some((tr) => tr.docChanged))) {
					noteSearchAnchor = transaction.mapping.map(noteSearchAnchor);
					for (const tr of appendedTransactions) noteSearchAnchor = tr.mapping.map(noteSearchAnchor);
					scheduleNoteSearch();
				}
				// Batch toolbar state updates to once per frame - avoids ~35 isActive() calls per transaction during selection drag
				if (!editorStateRaf) {
					editorStateRaf = requestAnimationFrame(() => {
						editorStateRaf = 0;
						editorState++;
					});
				}
				// On mobile, only check menus when they're already open or user just typed trigger char
				if (!isMobile || slashMenu || slashTypedByUser) updateSlashMenu();
				if (!isMobile || wikiLinkMenu || wikiLinkTypedByUser) updateWikiLinkMenu();
				if (!isMobile || taskMetaMenu || taskMetaTypedByUser) updateTaskMetaMenu();
				// Detect when cursor leaves a wiki-link mark and trigger rename check
				if ($appConfig?.enable_wiki_links && editor) {
					const curMarks = editor.state.selection.$from.marks();
					const curWikiMark = curMarks.find((m: any) => m.type.name === 'wikiLink') ?? null;
					if (prevCursorWikiMark && !curWikiMark) {
						checkWikiLinkRenames();
					}
					prevCursorWikiMark = curWikiMark;
				}
			},
			onUpdate: ({ transaction, appendedTransactions }) => {
				// setEditable emits update without changing the document.
				if (!transaction.docChanged && !appendedTransactions.some((tr) => tr.docChanged)) return;
				if (ignoreNextUpdate) {
					ignoreNextUpdate = false;
					return;
				}
				markEdited();
				if (showInfo) scheduleCounts();
			},
		});
		// TipTap normalizes a newly created document on its first transaction (it adds a paragraph
		// after a final list, for example). Run that now so it is not taken for the user's first
		// edit, which would save, and possibly reformat, a note that was only opened.
		ignoreNextUpdate = true;
		editor.view.dispatch(editor.state.tr);
		ignoreNextUpdate = false;
		clearEditorHistory();
		editorReady = true;
		if (showOutline) updateOutline();
		// Pre-load note titles for wiki-link autocomplete
		if ($appConfig?.enable_wiki_links) {
			refreshWikiLinkTitles();
		}
	}

	export async function toggleOutlinePanel() {
		if (showOutline) { await closeOutline(); return; }
		outlineTrigger = document.activeElement instanceof HTMLElement ? document.activeElement : null;
		showOutline = true;
		updateOutline();
		updateOutlineActive();
		const origin = captureOrigin();
		await tick();
		if (!showOutline || !isOriginCurrent(origin)) return;
		const button = outlineList?.querySelector<HTMLButtonElement>('button[aria-current="location"]')
			?? outlineList?.querySelector<HTMLButtonElement>('button')
			?? document.getElementById(outlinePanelId)?.querySelector<HTMLButtonElement>('.outline-close');
		button?.focus({ preventScroll: true });
	}

	export function toggleHistoryPanel() {
		toggleHistory();
	}

	export function triggerAiMenu() {
		openAiMenu();
	}

	export function toggleGraphView() {
		showGraph = !showGraph;
	}

	export function addLinkFromToolbar() {
		if (!editor) return;
		const { from, to } = editor.state.selection;
		linkSelectionFrom = from;
		linkSelectionTo = to;
		const previousUrl = editor.getAttributes('link').href || '';
		linkModalUrl = decodeURIComponent(previousUrl);
		linkSuggestIndex = 0;
		linkModal = true;
		prepareLinkModal();
	}

	let linkOrigin: EditorOrigin | null = null;

	function prepareLinkModal() {
		const origin = captureOrigin();
		linkOrigin = origin;
		if (!origin.vault) return;
		// The wiki-link cache is kept current by file events; skip a vault-wide scan when it is loaded.
		if ($appConfig?.enable_wiki_links && wikiTitleVault === origin.vault && !wikiTitleRefresh && wikiLinkTitlesCache.length) {
			linkSuggestTitles = [...wikiLinkTitlesCache];
		} else getAllNoteTitles(origin.vault).then(t => {
			if (linkModal && linkOrigin === origin && isOriginCurrent(origin)) linkSuggestTitles = t;
		}).catch(() => {});
		tick().then(() => {
			if (linkModal && linkOrigin === origin && isOriginCurrent(origin)) linkModalInput?.focus();
		});
	}

	function linkModalConfirm() {
		if (!editor || !linkOrigin || !canInsert(linkOrigin) || !isOriginCurrent(linkOrigin, true)) return;
		let url = linkModalUrl.trim();
		if (url === '') {
			editor.chain().focus().setTextSelection({ from: linkSelectionFrom, to: linkSelectionTo }).extendMarkRange('link').unsetLink().run();
		} else {
			if (url && !/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(url) && !url.startsWith('/') && !url.startsWith('#') && !url.endsWith('.md')) {
				url = 'https://' + url;
			}
			// Store raw URL - encoding is handled during markdown serialization/parsing
			const href = url.replace(/[()]/g, (c) => encodeURIComponent(c));
			if (linkSelectionFrom === linkSelectionTo) {
				const text = linkModalDisplayText || url.replace(/\.md$/, '').split('/').pop() || url;
				editor.chain().focus().setTextSelection(linkSelectionFrom).insertContent({
					type: 'text',
					text,
					marks: [{ type: 'link', attrs: { href } }],
				}).run();
			} else {
				editor.chain().focus().setTextSelection({ from: linkSelectionFrom, to: linkSelectionTo }).setMark('link', { href }).run();
			}
		}
		linkModal = false;
		linkModalUrl = '';
		linkModalDisplayText = '';
	}

	function linkModalSelectNote(entry: NoteTitleEntry) {
		// Build a relative .md path from the selected note and confirm immediately.
		// entry.path is vault-relative (e.g. "folder/note.md"); currentNote is absolute.
		const vaultRoot = $appConfig?.active_vault;
		const currentNote = $activeNotePath;
		if (vaultRoot && currentNote) {
			// Strip vault root from currentNote to get its relative path
			const normCurrent = currentNote.replace(/\\/g, '/');
			const normRoot = vaultRoot.replace(/\\/g, '/');
			const currentRel = normCurrent.startsWith(normRoot + '/') ? normCurrent.slice(normRoot.length + 1) : normCurrent;
			const noteDir = currentRel.substring(0, currentRel.lastIndexOf('/'));
			// entry.path is already relative
			const targetParts = entry.path.split('/');
			const currentParts = noteDir ? noteDir.split('/') : [];
			let common = 0;
			while (common < targetParts.length && common < currentParts.length && targetParts[common] === currentParts[common]) common++;
			const ups = currentParts.length - common;
			linkModalUrl = (ups > 0 ? '../'.repeat(ups) : './') + targetParts.slice(common).join('/');
		} else {
			linkModalUrl = entry.title + '.md';
		}
		linkModalDisplayText = entry.title;
		linkModalConfirm();
	}

	function linkModalCancel() {
		linkModal = false;
		linkModalUrl = '';
		linkModalDisplayText = '';
		editor?.chain().focus().run();
	}

	function insertTable(rows: number, cols: number) {
		if (!editor) return;
		editor.chain().focus().insertTable({ rows, cols, withHeaderRow: true }).run();
		tablePickerOpen = false;
		tablePickerHover = { rows: 0, cols: 0 };
	}

	function setTextColor(color: string) {
		if (!editor) return;
		if (color === '') {
			editor.chain().focus().unsetColor().run();
		} else {
			editor.chain().focus().setColor(color).run();
		}
		colorDropdown = false;
	}

	function setHighlightColor(color: string) {
		if (!editor) return;
		if (color === '') {
			editor.chain().focus().unsetHighlight().run();
		} else {
			editor.chain().focus().setHighlight({ color }).run();
		}
		highlightDropdown = false;
	}

	function handleEditorClick(event: MouseEvent) {
		const target = event.target as HTMLElement;

		const wikiLinkEl = target.closest('span[data-wiki-link]') as HTMLElement | null;
		if (wikiLinkEl) {
			imageToolbar = null;
			event.preventDefault();
			event.stopPropagation();
			const path = wikiLinkEl.getAttribute('data-path') || '';
			const title = wikiLinkEl.textContent || wikiLinkEl.getAttribute('data-title') || '';
			navigateToWikiLink(path, title, event);
			return;
		}

		if (target.tagName === 'IMG' && editor) {
			event.preventDefault();
			event.stopPropagation();
			const pos = editor.view.posAtDOM(target, 0);
			// If toolbar is already open for this image, close it
			if (imageToolbar && imageToolbar.pos === pos) {
				imageToolbar = null;
				return;
			}
			const node = editor.state.doc.nodeAt(pos);
			const currentSize = node?.attrs.size || 'full';
			const imgSrc = node?.attrs.src || (target as HTMLImageElement).src || '';
			const imgAlt = node?.attrs.alt || (target as HTMLImageElement).alt || '';
			const imgTitle = node?.attrs.title || '';
			const toolbarW = isAndroid ? 188 : isMobile ? 130 : 250;
			const toolbarH = isAndroid ? 48 : 38;
			const x = Math.max(8, Math.min(event.clientX, window.innerWidth - toolbarW - 8));
			const y = Math.max(8, Math.min(event.clientY, window.innerHeight - toolbarH - 8));
			const inGrid = editor.state.doc.resolve(pos).parent.type.name === 'imageGrid';
			imageToolbar = { pos, x, y, size: currentSize, src: imgSrc, alt: imgAlt, title: imgTitle, inGrid };
			// Move cursor after the image to clear ProseMirror's node selection highlight
			const afterPos = pos + (node?.nodeSize || 1);
			editor.chain().setTextSelection(afterPos).run();
			return;
		}

		imageToolbar = null;
	}

	function openImageViewer() {
		if (!imageToolbar) return;
		imageViewer = { src: imageToolbar.src, alt: imageToolbar.alt };
		imageToolbar = null;
	}

	/**
	 * The image toolbar remembers its image by position. Undo, redo or any other edit made while it
	 * is open can leave a different node there, and its buttons would then change that one instead.
	 */
	function closeStaleImageToolbar() {
		if (!editor || !imageToolbar) return;
		const { pos, src } = imageToolbar;
		const node = pos < editor.state.doc.content.size ? editor.state.doc.nodeAt(pos) : null;
		if (node?.type.name === 'image' && node.attrs.src === src) return;
		imageToolbar = null;
		imageTitleModal = null;
	}

	function setImageSize(size: string) {
		if (!editor || !imageToolbar) return;
		const { pos } = imageToolbar;
		const tr = editor.state.tr.setNodeAttribute(pos, 'size', size);
		editor.view.dispatch(tr);
		if (imageToolbar) imageToolbar = { ...imageToolbar, size };
	}

	function openImageTitleModal() {
		if (!editor || !imageToolbar) return;
		imageTitleModal = { title: editor.state.doc.nodeAt(imageToolbar.pos)?.attrs.title || imageToolbar.title };
	}

	/** Set or clear the toolbar image's hover text, saved as `![alt](src "title")`. */
	function commitImageTitleModal() {
		if (!editor || !imageToolbar || !imageTitleModal) return;
		const node = editor.state.doc.nodeAt(imageToolbar.pos);
		if (node?.type.name !== 'image') { imageTitleModal = null; return; }
		const title = imageTitleModal.title.trim();
		editor.view.dispatch(editor.state.tr.setNodeAttribute(imageToolbar.pos, 'title', title || null));
		if (imageToolbar) imageToolbar = { ...imageToolbar, title };
		imageTitleModal = null;
	}

	function cancelImageTitleModal() {
		imageTitleModal = null;
	}

	function getImageAbsPath(src: string): string {
		const assetPath = assetUrlToLocalPath(src);
		if (assetPath !== null) return assetPath;
		return resolveFileReference(src, $activeNotePath, currentNoteAssetRoot()).path;
	}

	async function copyImageToClipboard() {
		if (!imageToolbar) return;
		const absPath = getImageAbsPath(imageToolbar.src);
		imageToolbar = null;
		copyToast = 'copying';
		// Yield to let Svelte render the "Copying..." toast before blocking on IPC
		await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));
		try {
			await copyImageToClipboardCmd(absPath);
			copyToast = 'done';
		} catch (e) {
			console.error('Failed to copy image:', e);
			copyToast = null;
			return;
		}
		setTimeout(() => { copyToast = null; }, 1000);
	}

	function openImageInApp() {
		if (!imageToolbar) return;
		const absPath = getImageAbsPath(imageToolbar.src);
		openFile(absPath).catch(e => console.error('Failed to open image:', e));
		imageToolbar = null;
	}

	function handleEditorContextMenu(event: MouseEvent) {
		const target = event.target as HTMLElement;
		const anchor = target.closest('a');
		if (anchor) {
			const href = anchor.getAttribute('href');
			if (href) {
				event.preventDefault();
				event.stopPropagation();
				let lx = event.clientX;
				let ly = event.clientY;
				const lw = 200, lh = 200;
				if (lx + lw > window.innerWidth) lx = window.innerWidth - lw - 8;
				if (ly + lh > window.innerHeight) ly = window.innerHeight - lh - 8;
				if (lx < 4) lx = 4;
				if (ly < 4) ly = 4;
				linkContextMenu = { x: lx, y: ly, href, anchor };
				return;
			}
		}
		// Check if inside a table cell
		const cell = target.closest('td, th');
		if (cell) {
			event.preventDefault();
			event.stopPropagation();
			let x = event.clientX;
			let y = event.clientY;
			const menuWidth = 220;
			const menuHeight = 600;
			if (x + menuWidth > window.innerWidth) x = window.innerWidth - menuWidth - 8;
			if (y + menuHeight > window.innerHeight) y = window.innerHeight - menuHeight - 8;
			if (x < 4) x = 4;
			if (y < 4) y = 4;
			let hasStyling = false;
			if (editor) {
				const pos = editor.view.posAtDOM(event.target as Node, 0);
				let resolved = editor.state.doc.resolve(pos);
				for (let d = resolved.depth; d >= 0; d--) {
					if (resolved.node(d).type.name === 'table') {
						resolved.node(d).descendants((n: any) => {
							if (n.type.name === 'tableCell' || n.type.name === 'tableHeader') {
								if (n.attrs.backgroundColor || n.attrs.colwidth || (n.attrs.colspan > 1) || (n.attrs.rowspan > 1)) hasStyling = true;
							}
							return true;
						});
						break;
					}
				}
			}
			tableContextMenu = { x, y, hasStyling };
			return;
		}
		// Show text context menu for any right-click in editor
		event.preventDefault();
		event.stopPropagation();
		// Position menu, adjusting if it would overflow the viewport
		let x = event.clientX;
		let y = event.clientY;
		const menuWidth = 220;
		const menuHeight = 740;
		if (x + menuWidth > window.innerWidth) x = window.innerWidth - menuWidth - 8;
		if (y + menuHeight > window.innerHeight) y = window.innerHeight - menuHeight - 8;
		if (x < 4) x = 4;
		if (y < 4) y = 4;
		const submenuWidth = 150;
		const submenuLeft = x + menuWidth + submenuWidth > window.innerWidth;
		textContextMenu = { x, y, submenuLeft };
	}

	function closeTextContextMenu() {
		textContextMenu = null;
		ctxHeadingSubmenu = false;
	}

	function closeTableContextMenu() {
		tableContextMenu = null;
	}

	function tblAddRowBefore() { editor?.chain().focus().addRowBefore().run(); closeTableContextMenu(); }
	function tblAddRowAfter() { editor?.chain().focus().addRowAfter().run(); closeTableContextMenu(); }
	function tblDeleteRow() { editor?.chain().focus().deleteRow().run(); closeTableContextMenu(); }
	function tblAddColBefore() { editor?.chain().focus().addColumnBefore().run(); closeTableContextMenu(); }
	function tblAddColAfter() { editor?.chain().focus().addColumnAfter().run(); closeTableContextMenu(); }
	function tblDeleteCol() { editor?.chain().focus().deleteColumn().run(); closeTableContextMenu(); }
	function tblMergeCells() { editor?.chain().focus().mergeCells().run(); closeTableContextMenu(); }
	function tblSplitCell() { editor?.chain().focus().splitCell().run(); closeTableContextMenu(); }
	function tblToggleHeaderRow() { editor?.chain().focus().toggleHeaderRow().run(); closeTableContextMenu(); }
	function tblToggleHeaderCol() { editor?.chain().focus().toggleHeaderColumn().run(); closeTableContextMenu(); }
	function tblDeleteTable() { editor?.chain().focus().deleteTable().run(); closeTableContextMenu(); }
	function tblSetCellColor(color: string) {
		if (!editor) return;
		if (color === '') {
			editor.chain().focus().setCellAttribute('backgroundColor', null).run();
		} else {
			editor.chain().focus().setCellAttribute('backgroundColor', color).run();
		}
		closeTableContextMenu();
	}

	async function ctxCut() {
		if (!editor) return;
		const origin = captureOrigin();
		if (!canInsert(origin)) return;
		const { from, to } = editor.state.selection;
		if (from === to) { closeTextContextMenu(); return; }
		const text = editor.state.doc.textBetween(from, to, '\n');
		await navigator.clipboard.writeText(text);
		if (!canInsert(origin) || !isOriginCurrent(origin, true) || !editor) return;
		editor.commands.deleteRange({ from, to });
		closeTextContextMenu();
	}

	async function ctxCopy() {
		if (!editor) return;
		const origin = captureOrigin();
		const { from, to } = editor.state.selection;
		if (from === to) { closeTextContextMenu(); return; }
		const text = editor.state.doc.textBetween(from, to, '\n');
		await navigator.clipboard.writeText(text);
		if (isOriginCurrent(origin)) closeTextContextMenu();
	}

	/**
	 * Replays clipboard data as a paste event on the editor DOM, so it takes the same path as
	 * Ctrl/Cmd+V. ProseMirror's paste handler then runs every `handlePaste` prop (file paste,
	 * callout paste, link paste), `transformPastedHTML`, the clipboard parsers and TipTap's paste rules.
	 */
	function replayPaste(payload: ClipboardPastePayload): boolean {
		if (!editor) return false;
		const data = new DataTransfer();
		if (payload.text) data.setData('text/plain', payload.text);
		if (payload.html) data.setData('text/html', payload.html);
		for (const [type, value] of payload.strings) data.setData(type, value);
		for (const file of payload.files) data.items.add(file);
		const event = new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true });
		editor.view.focus();
		editor.view.dom.dispatchEvent(event);
		return event.defaultPrevented;
	}

	async function readWebClipboard(): Promise<ClipboardPastePayload | null> {
		try {
			const payload = await collectClipboardPayload(await navigator.clipboard.read());
			return isEmptyPayload(payload) ? null : payload;
		} catch (e) {
			// WebKitGTK (bug #218519) often cannot read content copied from other applications.
			console.warn('Web clipboard read failed, using the native clipboard:', e);
			return null;
		}
	}

	async function readNativeClipboard(): Promise<{ text?: string | null; html?: string | null }> {
		try {
			return await readClipboardContents();
		} catch (e) {
			console.error('Native clipboard read failed:', e);
			return {};
		}
	}

	async function ctxPaste() {
		// insertClipboardImage() saves the attachment before inserting it, so an
		// unguarded paste here would leave an orphan file behind when the editor
		// cannot accept the insertion.
		if (!editor || $readOnly || $viewerNote) { closeTextContextMenu(); return; }
		const origin = captureOrigin();
		let payload = await readWebClipboard();
		if (!canInsert(origin) || !editor) return;
		// WebKitGTK (bug #218519) often gives nothing, or only plain text, for content from other
		// apps. Fill in text and HTML from the native clipboard so formatting survives like Ctrl+V.
		if (needsNativeClipboard(payload)) {
			payload = withNativeClipboard(payload, await readNativeClipboard());
			if (!canInsert(origin) || !editor) return;
		}
		// Replaying the event runs Ctrl+V's handlers, including the WebKitGTK image fallback for a
		// clipboard holding only an image (or a bare <img>).
		if (payload && !isEmptyPayload(payload) && replayPaste(payload)) {
			closeTextContextMenu();
			return;
		}
		// Nothing textual at all: the clipboard may still hold a bitmap only the native reader sees.
		await insertClipboardImage(origin);
		if (isOriginCurrent(origin)) closeTextContextMenu();
	}

	function ctxSelectAll() {
		if (!editor) return;
		editor.chain().focus().selectAll().run();
		closeTextContextMenu();
	}

	let ctxHeadingSubmenu = $state(false);

	function ctxSetHeading(level: number) {
		editor?.chain().focus().toggleHeading({ level: level as 1 | 2 | 3 | 4 }).run();
		closeTextContextMenu();
	}

	function ctxSetParagraph() {
		editor?.chain().focus().setParagraph().run();
		closeTextContextMenu();
	}

	function ctxBold() {
		editor?.chain().focus().toggleBold().run();
		closeTextContextMenu();
	}

	function ctxItalic() {
		editor?.chain().focus().toggleItalic().run();
		closeTextContextMenu();
	}

	function ctxUnderline() {
		editor?.chain().focus().toggleUnderline().run();
		closeTextContextMenu();
	}

	function ctxStrike() {
		editor?.chain().focus().toggleStrike().run();
		closeTextContextMenu();
	}

	function ctxLink() {
		closeTextContextMenu();
		addLinkFromToolbar();
	}

	function ctxHighlight() {
		editor?.chain().focus().toggleHighlight({ color: highlightColors[0].value }).run();
		closeTextContextMenu();
	}

	function ctxClearFormatting() {
		if (editor) clearFormatting(editor);
		closeTextContextMenu();
	}

	function ctxCode() {
		editor?.chain().focus().toggleCode().run();
		closeTextContextMenu();
	}

	function ctxCodeBlock() {
		editor?.chain().focus().toggleCodeBlock().run();
		closeTextContextMenu();
	}

	function ctxBlockquote() {
		editor?.chain().focus().toggleBlockquote().run();
		closeTextContextMenu();
	}

	function ctxTimestamp() {
		insertTimestamp('datetime');
		closeTextContextMenu();
	}

	function openDetailsEl(el: HTMLElement) {
		if (!el.classList.contains('is-open')) {
			el.classList.add('is-open');
			(el.querySelector('[data-type="detailsContent"]') as HTMLElement | null)
				?.dispatchEvent(new Event('toggleDetailsContent'));
		}
	}

	function insertCallout(type = 'note') {
		if (!editor) return;
		editor.chain().focus().wrapIn('callout', { type, title: '', foldable: false, folded: false }).run();
	}

	function openCalloutTypeMenu(anchor: HTMLElement, onPick: (type: string) => void) {
		document.querySelectorAll('.callout-type-menu').forEach((el) => el.remove());
		const menu = document.createElement('div');
		menu.className = 'callout-type-menu';
		const close = () => {
			menu.remove();
			document.removeEventListener('mousedown', onDoc, true);
			document.removeEventListener('keydown', onKey, true);
			window.removeEventListener('scroll', close, true);
		};
		const onDoc = (e: MouseEvent) => { if (!menu.contains(e.target as Node)) close(); };
		const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') close(); };
		for (const item of CALLOUT_MENU) {
			const btn = document.createElement('button');
			btn.type = 'button';
			btn.className = 'callout-type-option';
			btn.innerHTML = `<span class="callout-type-icon" style="color: rgb(var(--callout-${item.type}))">${calloutIcon(item.type, 16)}</span><span>${item.label}</span>`;
			btn.addEventListener('mousedown', (e) => e.preventDefault());
			btn.addEventListener('click', () => { onPick(item.type); close(); });
			menu.appendChild(btn);
		}
		// "Custom…" fills the last grid slot: name any callout type (Obsidian round-trips it).
		const customBtn = document.createElement('button');
		customBtn.type = 'button';
		customBtn.className = 'callout-type-option';
		customBtn.innerHTML = '<span class="callout-type-icon" style="color: var(--text-secondary)"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="4" y1="21" x2="4" y2="14"/><line x1="4" y1="10" x2="4" y2="3"/><line x1="12" y1="21" x2="12" y2="12"/><line x1="12" y1="8" x2="12" y2="3"/><line x1="20" y1="21" x2="20" y2="16"/><line x1="20" y1="12" x2="20" y2="3"/><line x1="1" y1="14" x2="7" y2="14"/><line x1="9" y1="8" x2="15" y2="8"/><line x1="17" y1="16" x2="23" y2="16"/></svg></span><span>Custom…</span>';
		customBtn.addEventListener('mousedown', (e) => e.preventDefault());
		customBtn.addEventListener('click', () => {
			menu.classList.add('is-custom');
			menu.innerHTML = '';
			const input = document.createElement('input');
			input.className = 'callout-type-custom-input';
			input.placeholder = 'Type name, then Enter';
			input.spellcheck = false;
			input.addEventListener('keydown', (e) => {
				if (e.key === 'Enter') {
					e.preventDefault();
					const v = input.value.trim().toLowerCase().replace(/[^\w-]/g, '');
					if (v) onPick(v);
					close();
				} else if (e.key === 'Escape') {
					e.preventDefault();
					close();
				}
			});
			menu.appendChild(input);
			input.focus();
		});
		menu.appendChild(customBtn);
		document.body.appendChild(menu);
		const r = anchor.getBoundingClientRect();
		menu.style.top = `${Math.min(r.bottom + 4, window.innerHeight - menu.offsetHeight - 8)}px`;
		menu.style.left = `${Math.min(r.left, window.innerWidth - menu.offsetWidth - 8)}px`;
		setTimeout(() => {
			document.addEventListener('mousedown', onDoc, true);
			document.addEventListener('keydown', onKey, true);
			window.addEventListener('scroll', close, true);
		}, 0);
	}

	function handleGridInput(event: Event) {
		const input = event.currentTarget as HTMLInputElement;
		const origin = assetPickerOrigins.get(input);
		assetPickerOrigins.delete(input);
		const files = Array.from(input.files ?? []).filter((file) => file.type.startsWith('image/'));
		input.value = '';
		if (origin && files.length) void insertImageGrid(files, origin);
	}

	/** Save the picked images and insert them as one grid (a single image is inserted normally). */
	async function insertImageGrid(files: File[], origin: EditorOrigin) {
		if (files.length === 1 || origin.sourceMode) {
			for (const file of files) await insertFile(file, origin);
			return;
		}
		const sources: string[] = [];
		for (const file of files) {
			if (!canInsert(origin)) return;
			try {
				const data = Array.from(new Uint8Array(await file.arrayBuffer()));
				if (!canInsert(origin)) return;
				sources.push(resolveImageSrc(encodeMarkdownPath(await saveImage(origin.vault!, file.name, data))));
			} catch (e) {
				console.error('Failed to insert image:', e);
			}
		}
		if (!canInsert(origin) || !editor || sources.length === 0) return;
		editor.commands.insertContent(sources.length === 1
			? { type: 'image', attrs: { src: sources[0] } }
			: { type: 'imageGrid', attrs: { cols: defaultGridCols(sources.length), layout: 'grid' },
				content: sources.map((src) => ({ type: 'image', attrs: { src } })) });
	}

	function countBlockImages(node: ProseMirrorNode): number {
		if (node.type.name === 'imageGrid') return node.childCount;
		if (!isImageNode(node)) return 0;
		let count = 0;
		node.forEach((child) => { if (child.type.name === 'image') count++; });
		return count;
	}

	/** Group the toolbar's image and its neighboring image-only blocks (and grids) into one grid. */
	function wrapNearbyImagesInGrid() {
		if (!editor || !imageToolbar) return;
		const { state } = editor;
		const blocks: { node: ProseMirrorNode; offset: number }[] = [];
		state.doc.forEach((node, offset) => blocks.push({ node, offset }));
		const { pos } = imageToolbar;
		const target = blocks.findIndex((block) => pos >= block.offset && pos < block.offset + block.node.nodeSize);
		const range = expandImageRange(blocks.map((block) => ({
			isImage: countBlockImages(block.node) > 0,
			imageCount: countBlockImages(block.node),
			isEmpty: block.node.type.name === 'paragraph' && block.node.childCount === 0,
		})), target);
		if (!range) return;
		const images: ProseMirrorNode[] = [];
		for (let i = range.start; i <= range.end; i++) {
			blocks[i].node.forEach((child) => { if (child.type.name === 'image') images.push(child); });
		}
		const grid = state.schema.nodes.imageGrid.create({ cols: defaultGridCols(images.length), layout: 'grid' }, images);
		const from = blocks[range.start].offset;
		const to = blocks[range.end].offset + blocks[range.end].node.nodeSize;
		editor.view.dispatch(state.tr.replaceWith(from, to, grid));
		imageToolbar = null;
	}

	/**
	 * Replace a grid with one image paragraph per image. At the top level each image gets the empty
	 * paragraph that stands for the blank line above it, so the images are saved a blank line apart
	 * instead of being glued to the paragraph above (the reverse of grouping them).
	 */
	function ungroupImageGridAt(gridPos: number) {
		if (!editor) return;
		const { state } = editor;
		const grid = state.doc.nodeAt(gridPos);
		if (!grid || grid.type.name !== 'imageGrid') return;
		const resolved = state.doc.resolve(gridPos);
		const before = resolved.nodeBefore;
		const topLevel = resolved.depth === 0;
		const paragraphs: ProseMirrorNode[] = [];
		grid.forEach((image, _offset, index) => {
			const blankLineAbove = index > 0 || (!!before && !(before.type.name === 'paragraph' && before.childCount === 0));
			if (topLevel && blankLineAbove) paragraphs.push(state.schema.nodes.paragraph.create());
			paragraphs.push(state.schema.nodes.paragraph.create(null, image));
		});
		editor.view.dispatch(state.tr.replaceWith(gridPos, gridPos + grid.nodeSize, paragraphs));
		imageToolbar = null;
	}

	function gridChildren(grid: ProseMirrorNode): ProseMirrorNode[] {
		const children: ProseMirrorNode[] = [];
		grid.forEach((child) => children.push(child));
		return children;
	}

	/** Move the toolbar's image one place left or right inside its grid. */
	function moveGridImage(direction: -1 | 1) {
		if (!editor || !imageToolbar?.inGrid) return;
		const { state } = editor;
		const resolved = state.doc.resolve(imageToolbar.pos);
		const grid = resolved.parent;
		if (grid.type.name !== 'imageGrid') return;
		const index = resolved.index();
		const next = index + direction;
		if (next < 0 || next >= grid.childCount) return;
		const children = gridChildren(grid);
		[children[index], children[next]] = [children[next], children[index]];
		const gridPos = resolved.before();
		let newPos = gridPos + 1;
		for (let i = 0; i < next; i++) newPos += children[i].nodeSize;
		// Follow the image before the change lands, so the toolbar is not taken for a stale one.
		imageToolbar = { ...imageToolbar, pos: newPos };
		editor.view.dispatch(state.tr.replaceWith(gridPos, gridPos + grid.nodeSize, grid.type.create(grid.attrs, children)));
	}

	/** Drag-and-drop: move an image within a grid or into another grid. An emptied source grid is removed. */
	function moveGridImageByIndex(fromGridPos: number, fromIndex: number, toGridPos: number, toIndex: number) {
		if (!editor) return;
		const { state } = editor;
		const source = state.doc.nodeAt(fromGridPos);
		if (!source || source.type.name !== 'imageGrid' || fromIndex < 0 || fromIndex >= source.childCount) return;
		const sourceChildren = gridChildren(source);
		if (fromGridPos === toGridPos) {
			const target = reorderTarget(fromIndex, toIndex, sourceChildren.length);
			if (target === null) return;
			const [moved] = sourceChildren.splice(fromIndex, 1);
			sourceChildren.splice(target, 0, moved);
			editor.view.dispatch(state.tr.replaceWith(fromGridPos, fromGridPos + source.nodeSize, source.type.create(source.attrs, sourceChildren)));
		} else {
			const destination = state.doc.nodeAt(toGridPos);
			if (!destination || destination.type.name !== 'imageGrid') return;
			const [moved] = sourceChildren.splice(fromIndex, 1);
			const tr = state.tr;
			if (sourceChildren.length === 0) tr.delete(fromGridPos, fromGridPos + source.nodeSize);
			else tr.replaceWith(fromGridPos, fromGridPos + source.nodeSize, source.type.create(source.attrs, sourceChildren));
			const mapped = tr.mapping.map(toGridPos);
			const destinationChildren = gridChildren(destination);
			destinationChildren.splice(Math.max(0, Math.min(destinationChildren.length, toIndex)), 0, moved);
			tr.replaceWith(mapped, mapped + destination.nodeSize, destination.type.create(destination.attrs, destinationChildren));
			editor.view.dispatch(tr);
		}
		imageToolbar = null;
	}

	function insertDetails() {
		if (!editor) return;
		editor.chain().focus().setDetails().run();
		const origin = captureOrigin();
		requestAnimationFrame(() => {
			if (!isOriginCurrent(origin, true)) return;
			if (!editor) return;
			const domPos = editor.view.domAtPos(editor.state.selection.from);
			let node = domPos.node as HTMLElement;
			if (node.nodeType === 3) node = node.parentElement as HTMLElement;
			const detailsEl = node.closest('[data-type="details"]') as HTMLElement | null;
			if (detailsEl) openDetailsEl(detailsEl);
			// Sync open: true into the document so it saves with the note
			editor.chain().updateAttributes('details', { open: true }).run();
		});
	}

	function ctxDetails() {
		insertDetails();
		closeTextContextMenu();
	}

	function ctxCallout() {
		insertCallout('note');
		closeTextContextMenu();
	}

	function handleMixedListToggle(targetListName: MixedListName): boolean {
		if (!editor) return false;
		const { selection, schema } = editor.state;
		const { $from: fromResolved, $to: toResolved } = selection;
		let listDepth = -1;

		for (let depth = fromResolved.depth; depth > 0; depth--) {
			const nodeName = fromResolved.node(depth).type.name;
			// An ordered list cannot contain task items. Keep it ordered instead of lifting its item.
			if (nodeName === 'orderedList') return targetListName === 'taskList';
			if (nodeName === 'bulletList' || nodeName === 'taskList') {
				listDepth = depth;
				break;
			}
		}

		if (listDepth < 0 || toResolved.depth < listDepth) return false;
		const listNode = fromResolved.node(listDepth);
		if (listNode.type.name === targetListName || toResolved.node(listDepth) !== listNode) return false;

		const convertedList = convertListNode(schema, listNode, targetListName);
		if (!convertedList) return false;
		const listPos = fromResolved.before(listDepth);
		const transaction = editor.state.tr.replaceWith(
			listPos,
			listPos + listNode.nodeSize,
			convertedList
		);
		transaction.setSelection(TextSelection.create(transaction.doc, selection.from, selection.to));
		editor.view.dispatch(transaction.scrollIntoView());
		editor.view.focus();
		return true;
	}

	function toggleBulletList(): boolean {
		if (!editor) return false;
		return handleMixedListToggle('bulletList') || editor.chain().focus().toggleBulletList().run();
	}

	function toggleTaskList(): boolean {
		if (!editor) return false;
		return handleMixedListToggle('taskList') || editor.chain().focus().toggleTaskList().run();
	}

	function ctxBulletList() {
		toggleBulletList();
		closeTextContextMenu();
	}

	function ctxOrderedList() {
		editor?.chain().focus().toggleOrderedList().run();
		closeTextContextMenu();
	}

	function ctxTaskList() {
		toggleTaskList();
		closeTextContextMenu();
	}

	// ── AI Actions ──

	function openAiMenu() {
		if ((!$sourceMode && !editor) || $readOnly || $viewerNote) return;
		closeAiMenu();
		aiOrigin = captureOrigin();
		closeTextContextMenu();

		if ($sourceMode) {
			const { from: start, to: end } = source.state.selection.main;
			const selected = source.state.doc.sliceString(start, end);
			aiResult = null;
			aiError = null;
			aiLoading = false;
			aiShowCustom = false;
			aiTranslateMenu = false;
			aiCustomPrompt = '';
			aiMediaPlaceholders = new Map();
			aiOriginalMarkdown = '';
			aiEmptyNote = false;
			if (selected.trim()) {
				aiSelectionFrom = start;
				aiSelectionTo = end;
				aiSelectedText = selected;
				aiWholeNote = false;
			} else {
				const fullMd = restoreTitleH1(source.state.doc.toString());
				if (!fullMd.trim()) {
					aiEmptyNote = true;
					aiWholeNote = true;
					aiSelectedText = '';
				} else {
					aiWholeNote = true;
					aiSelectedText = fullMd.trim();
				}
				aiSelectionFrom = 0;
				aiSelectionTo = 0;
			}
			aiMenu = isMobile ? { x: 0, y: 0 } : { x: Math.max(4, window.innerWidth / 2 - 110), y: 100 };
			return;
		}
		if (!editor) return;

		editor.commands.focus();

		const { from, to } = editor.state.selection;
		const hasSelection = from !== to;

		if (hasSelection) {
			const selectedText = editor.state.doc.textBetween(from, to, '\n');
			if (!selectedText.trim()) return;
			aiSelectionFrom = from;
			aiSelectionTo = to;
			aiSelectedText = selectedText;
			aiWholeNote = false;
		} else {
			const fullMarkdown = editorToMarkdown();
			if (!fullMarkdown.trim()) {
				aiEmptyNote = true;
				aiWholeNote = true;
				aiSelectedText = '';
				aiSelectionFrom = 0;
				aiSelectionTo = 0;
			} else {
				aiEmptyNote = false;
				aiSelectionFrom = 0;
				aiSelectionTo = editor.state.doc.content.size;
				aiOriginalMarkdown = fullMarkdown;

				// Replace images, PDF embeds, and HTML tags with placeholders so AI doesn't mangle them
				const placeholders = new Map<string, string>();
				let idx = 0;
				let textForAi = fullMarkdown;
				// Images: ![alt](src)
				textForAi = textForAi.replace(/!\[[^\]]*\]\([^)]*\)/g, (match) => {
					const key = `__MEDIA_${idx++}__`;
					placeholders.set(key, match);
					return key;
				});
				// PDF embeds
				textForAi = textForAi.replace(/<div[^>]*class="pdf-embed"[^>]*>[\s\S]*?<\/div>/gi, (match) => {
					const key = `__MEDIA_${idx++}__`;
					placeholders.set(key, match);
					return key;
				});
				// Inline HTML img tags
				textForAi = textForAi.replace(/<img[^>]*\/?>/gi, (match) => {
					const key = `__MEDIA_${idx++}__`;
					placeholders.set(key, match);
					return key;
				});
				aiMediaPlaceholders = placeholders;
				aiSelectedText = textForAi.trim();
				if (!aiSelectedText) return;
				aiWholeNote = true;
			}
		}

		aiResult = null;
		aiError = null;
		aiLoading = false;
		aiShowCustom = false;
		aiTranslateMenu = false;
		aiCustomPrompt = '';

		if (isMobile) {
			// Mobile: bottom sheet, no positioning needed
			aiMenu = { x: 0, y: 0 };
		} else if (hasSelection) {
			const coords = editor.view.coordsAtPos(from);
			let x = coords.left;
			let y = coords.top - 8;
			const menuWidth = 220;
			const menuHeight = 400;
			if (x + menuWidth > window.innerWidth) x = window.innerWidth - menuWidth - 8;
			if (y - menuHeight < 0) y = coords.bottom + 8;
			else y = y - menuHeight;
			if (y < 4) y = 4;
			aiMenu = { x, y };
		} else {
			// Center the menu in the editor area
			const editorRect = editorElement?.getBoundingClientRect();
			const menuWidth = 220;
			const menuHeight = 400;
			let x = editorRect ? editorRect.left + (editorRect.width - menuWidth) / 2 : window.innerWidth / 2 - menuWidth / 2;
			let y = editorRect ? editorRect.top + 60 : 100;
			if (x < 4) x = 4;
			if (y < 4) y = 4;
			aiMenu = { x, y };
		}
	}

	function closeAiMenu() {
		aiGeneration++;
		aiOrigin = null;
		if (aiStreamUnlisten) { aiStreamUnlisten(); aiStreamUnlisten = null; }
		aiMenu = null;
		aiLoading = false;
		aiResult = null;
		aiError = null;
		aiShowCustom = false;
		aiTranslateMenu = false;
		aiWholeNote = false;
		aiEmptyNote = false;
		aiOriginalMarkdown = '';
		aiMediaPlaceholders = new Map();
	}

	async function runAiAction(action: string, customPrompt?: string) {
		const origin = aiOrigin;
		if (!origin || !canInsert(origin)) return;
		if (!isOriginCurrent(origin, true)) {
			aiError = 'The note changed. Reopen AI Actions to use its current content.';
			return;
		}
		if (!aiEmptyNote && !aiSelectedText.trim()) return;
		// For empty notes, force action to 'custom' with a generate instruction
		if (aiEmptyNote) {
			action = 'custom';
			customPrompt = `Generate a note based on this prompt. Start your response with a title on the first line (no # prefix), then a blank line, then the content in markdown. Prompt: ${customPrompt || aiCustomPrompt}`;
			aiSelectedText = '(empty note)';
		}
		aiLoading = true;
		aiResult = '';
		aiError = null;
		aiShowCustom = false;
		aiTranslateMenu = false;

		// Cancel any previous stream listener
		if (aiStreamUnlisten) { aiStreamUnlisten(); aiStreamUnlisten = null; }

		const generation = ++aiGeneration;
		const requestId = crypto.randomUUID();
		const selectedText = aiSelectedText;
		const current = () => generation === aiGeneration && aiOrigin === origin && isOriginCurrent(origin);
		let stop: (() => void) | null = null;
		try {
			stop = await listen<AiStreamEvent>('ai-stream', (event) => {
				const data = event.payload;
				if (!current() || data.request_id !== requestId) return;
				if (data.event_type === 'text' && data.text) {
					aiResult = (aiResult ?? '') + data.text;
				} else if (data.event_type === 'done' || data.event_type === 'error') {
					if (data.event_type === 'error') aiError = data.error ?? 'Unknown error';
					aiLoading = false;
					aiStreamUnlisten = null;
					stop?.();
				}
			});
			if (!current()) { stop(); return; }
			aiStreamUnlisten = stop;
			await aiAsk(action, selectedText, customPrompt ?? null, requestId);
		} catch (e) {
			stop?.();
			if (!current()) return;
			aiError = String(e);
			aiLoading = false;
			aiStreamUnlisten = null;
		}
	}

	async function aiApplyResult() {
		const origin = aiOrigin;
		if (!origin || !aiResult || aiLoading || !canInsert(origin)) return;
		if (!isOriginCurrent(origin, true)) {
			aiError = 'The note changed. Reopen AI Actions before applying a new result.';
			return;
		}
		const generation = aiGeneration;
		const result = aiResult;
		if (origin.path && origin.noteId && !aiEmptyNote) {
			try {
				if (!(await forceSave()) || !isOriginCurrent(origin, true) || generation !== aiGeneration) return;
				await createVersion(origin.vault!, origin.path, origin.noteId);
			} catch (e) {
				console.error('Failed to create version before AI apply:', e);
			}
		}
		if (!isOriginCurrent(origin, true) || !canInsert(origin) || generation !== aiGeneration) return;
		if ($sourceMode) {
			if (aiEmptyNote) {
				const lines = result.split('\n');
				let title = lines[0]?.replace(/^#+\s*/, '').trim() || 'Untitled';
				const body = lines.slice(1).join('\n').replace(/^\n+/, '');
				if ($activeNote) $activeNote.meta.title = title;
				markEdited();
				source.replace(body);
			} else if (aiWholeNote) {
				source.replace(stripTitleH1(result));
			} else {
				source.replace(result, aiSelectionFrom, aiSelectionTo);
			}
			closeAiMenu();
			return;
		}
		if (!editor) return;
		if (aiEmptyNote) {
			// Parse title from first line, rest is content
			const lines = result.split('\n');
			let title = lines[0]?.replace(/^#+\s*/, '').trim() || 'Untitled';
			const body = lines.slice(1).join('\n').replace(/^\n+/, '');
			if ($activeNote) {
				$activeNote.meta.title = title;
				markEdited();
			}
			editor.commands.setContent(markdownToHtml(body));
		} else if (aiWholeNote) {
			// Restore media placeholders back to original markdown
			let finalMarkdown = result;
			for (const [key, original] of aiMediaPlaceholders) {
				finalMarkdown = finalMarkdown.replace(key, original);
			}
			// Replace entire document - convert markdown back to HTML for TipTap
			editor.commands.setContent(markdownToHtml(finalMarkdown));
		} else {
			// Replace the selected range with the AI result (convert markdown → HTML so TipTap renders it properly)
			const html = markdownToHtml(result);
			editor.commands.insertContentAt({ from: aiSelectionFrom, to: aiSelectionTo }, html);
		}
		closeAiMenu();
	}

	function aiDiscard() {
		closeAiMenu();
		if ($sourceMode) source.focus();
		else editor?.commands.focus();
	}

	function closeLinkContextMenu() {
		linkContextMenu = null;
	}

	/** Resolve a link href to an absolute .md note path, or null if not a note link. */
	function resolveNoteHref(href: string): string | null {
		if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(href)) return null;
		const path = resolveFileReference(href, $activeNotePath, currentNoteAssetRoot()).path;
		return path.endsWith('.md') ? path : null;
	}

	function linkMenuOpen() {
		if (!linkContextMenu) return;
		const href = linkContextMenu.href;
		closeLinkContextMenu();
		// Internal .md note link - navigate within the app
		const notePath = resolveNoteHref(href);
		if (notePath) {
			navigateToWikiLink(notePath, '');
			return;
		}
		if (href.startsWith('http://') || href.startsWith('https://') || href.startsWith('mailto:')) {
			openUrl(href).catch(console.error);
		} else {
			openFile(resolveHrefToAbsPath(href)).catch(console.error);
		}
	}

	function linkMenuCopy() {
		if (!linkContextMenu) return;
		navigator.clipboard.writeText(linkContextMenu.href).catch(console.error);
		closeLinkContextMenu();
	}

	function linkMenuEdit() {
		if (!linkContextMenu || !editor) return;
		const anchor = linkContextMenu.anchor;
		const href = linkContextMenu.href;
		closeLinkContextMenu();
		// Select the link text so the modal edits the right link
		const pos = editor.view.posAtDOM(anchor, 0);
		if (pos >= 0) {
			editor.chain().focus().setTextSelection(pos).extendMarkRange('link').run();
		}
		const { from, to } = editor.state.selection;
		linkSelectionFrom = from;
		linkSelectionTo = to;
		linkModalUrl = decodeURIComponent(href);
		linkModalDisplayText = '';
		linkModal = true;
		prepareLinkModal();
	}

	function linkMenuRemove() {
		if (!linkContextMenu || !editor) return;
		const anchor = linkContextMenu.anchor;
		const pos = editor.view.posAtDOM(anchor, 0);
		if (pos >= 0) {
			editor.chain()
				.focus()
				.setTextSelection(pos)
				.extendMarkRange('link')
				.unsetLink()
				.run();
		}
		closeLinkContextMenu();
	}

	function resolveHrefToAbsPath(href: string): string {
		return resolveFileReference(href, $activeNotePath, currentNoteAssetRoot()).path;
	}

	function isFileLink(href: string): boolean {
		if (href.startsWith('http://') || href.startsWith('https://') || href.startsWith('mailto:')) return false;
		const ext = href.split('.').pop()?.toLowerCase() ?? '';
		return ['pdf', 'zip', 'rar', '7z', 'tar', 'gz', 'doc', 'docx', 'xls', 'xlsx', 'ppt', 'pptx', 'csv', 'txt', 'rtf', 'odt', 'ods', 'mp3', 'mp4', 'wav', 'avi', 'mov', 'mkv', 'exe', 'dmg', 'apk', 'iso', 'epub'].includes(ext);
	}

	async function linkMenuSaveAs() {
		if (!linkContextMenu) return;
		const href = linkContextMenu.href;
		closeLinkContextMenu();
		const absPath = resolveHrefToAbsPath(href);
		const filename = absPath.split('/').pop() || 'file';
		const dest = await saveDialog({ defaultPath: filename });
		if (dest) {
			try {
				await copyFileTo(absPath, dest);
			} catch (e) {
				console.error('Failed to save file:', e);
			}
		}
	}

	function handleFileDrop(event: DragEvent): boolean {
		const files = event.dataTransfer?.files;
		if (!files || files.length === 0) return false;
		event.preventDefault();
		const origin = captureOrigin();
		for (const file of Array.from(files)) void insertFile(file, origin);
		return true;
	}

	function handleFilePaste(event: ClipboardEvent): boolean {
		const items = event.clipboardData?.items;
		if (!items) return false;
		for (const item of Array.from(items)) {
			const file = item.getAsFile();
			if (!file) continue;
			event.preventDefault();
			void insertFile(file);
			return true;
		}
		// WebKitGTK fallback (bug #218519): WebKitGTK never exposes a clipboard image
		// through DataTransfer, so the loop above cannot fire on Linux and this is the
		// only path that can work there. The guard used to also require an empty
		// DataTransferItemList and no text/html, which no real image copy satisfies:
		// Chromium's "Copy image" writes an <img> HTML fragment alongside the bitmap
		// (but no text/plain), so the image was dropped and its remote URL pasted
		// instead. Key off text/plain, and accept HTML that is just an <img>.
		const hasText = event.clipboardData!.getData('text/plain');
		const html = event.clipboardData!.getData('text/html');
		const htmlIsBareImage = /^\s*(?:<meta[^>]*>\s*)?<img\b[^>]*>\s*$/i.test(html);
		if (!hasText && (!html || htmlIsBareImage)) {
			event.preventDefault();
			insertClipboardImage();
			return true;
		}
		return false;
	}

	/** Inserts the native clipboard image. Resolves true when there was an image to insert. */
	async function insertClipboardImage(origin = captureOrigin()): Promise<boolean> {
		if (!canInsert(origin)) return false;
		try {
			const data = await readClipboardImage();
			if (!canInsert(origin)) return false;
			await uploadAsset(origin, 'pasted-image.png', data, 'image');
			return true;
		} catch (e) {
			console.error('Clipboard image fallback failed:', e);
			return false;
		}
	}

	type AssetKind = 'image' | 'pdf' | 'file';

	async function uploadAsset(origin: EditorOrigin, name: string, data: number[], kind: AssetKind) {
		if (!canInsert(origin)) return;
		const relativePath = kind === 'image'
			? await saveImage(origin.vault!, name, data)
			: await saveAttachment(origin.vault!, name, data);
		if (!canInsert(origin)) return;
		const label = `${name} (${Math.round(data.length / 1024)} kB)`;
		// The stored name may contain #, %, spaces or parentheses; every use below is a URI or Markdown destination.
		const destination = encodeMarkdownPath(relativePath);
		if (origin.sourceMode) {
			const { from, to } = source.state.selection.main;
			const escaped = (kind === 'image' ? name : label).replace(/[[\]\\]/g, '\\$&');
			source.replace(`${kind === 'image' ? '!' : ''}[${escaped}](${destination}) `, from, to);
		} else if (editor) {
			// Async completion may insert into its origin, but must never take focus.
			if (kind === 'image') editor.commands.setImage({ src: resolveImageSrc(destination) });
			else if (kind === 'pdf' && !isMobile && $appConfig?.pdf_preview) {
				editor.commands.insertContent({ type: 'pdfEmbed', attrs: { src: destination, name } });
			} else {
				editor.commands.insertContent([
					{ type: 'text', text: label, marks: [{ type: 'link', attrs: { href: destination } }] },
					{ type: 'text', text: ' ' },
				]);
			}
		}
	}

	async function insertFile(file: File, origin = captureOrigin()) {
		if (!canInsert(origin)) return;
		try {
			const buffer = await file.arrayBuffer();
			if (!canInsert(origin)) return;
			await uploadAsset(origin, file.name, Array.from(new Uint8Array(buffer)),
				file.type.startsWith('image/') ? 'image' : file.type === 'application/pdf' ? 'pdf' : 'file');
		} catch (e) {
			console.error('Failed to insert attachment:', e);
		}
	}

	const assetPickerOrigins = new WeakMap<HTMLInputElement, EditorOrigin>();

	function openAssetPicker(id: string) {
		insertDropdown = false;
		const input = document.getElementById(id) as HTMLInputElement | null;
		if (!input) return;
		assetPickerOrigins.set(input, captureOrigin());
		input.click();
	}

	function handleAssetInput(event: Event) {
		const input = event.currentTarget as HTMLInputElement;
		const origin = assetPickerOrigins.get(input);
		assetPickerOrigins.delete(input);
		if (origin) for (const file of Array.from(input.files ?? [])) void insertFile(file, origin);
		input.value = '';
	}

	async function saveBlobImage(vault: string, blobUrl: string): Promise<string> {
		const resp = await fetch(blobUrl);
		if (!resp.ok) throw new Error(`Failed to read pasted image: ${resp.status}`);
		const blob = await resp.blob();
		const ext = blob.type.split('/')[1] || 'png';
		const buffer = await blob.arrayBuffer();
		return saveImage(vault, `pasted-image.${ext}`, Array.from(new Uint8Array(buffer)));
	}

	function replaceSavedBlob(origin: EditorOrigin, blobUrl: string, relativePath: string) {
		if (!isOriginCurrent(origin)) return;
		if ($sourceMode) {
			// Only asset resolution serializes here; ordinary CM edits retain Text.
			const text = source.state.doc.toString();
			const matches = [...text.matchAll(new RegExp(blobUrl.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'g'))];
			for (const match of matches.reverse()) source.replace(relativePath, match.index!, match.index! + blobUrl.length, true);
			return;
		}
		if (!editor) return;
		const tr = editor.state.tr;
		editor.state.doc.descendants((node, pos) => {
			if (node.type.name === 'image' && node.attrs.src === blobUrl) {
				tr.setNodeAttribute(pos, 'src', resolveImageSrc(relativePath));
			}
		});
		if (tr.docChanged) {
			// Resolving an image changes its representation, not the user's revision.
			ignoreNextUpdate = true;
			editor.view.dispatch(tr);
			ignoreNextUpdate = false;
		}
	}


	// Source mode toggle - only react to explicit user toggle, not note switches
	$effect(() => {
		const isSource = $sourceMode;
		// Only act if we have a loaded note, or one the rich editor could not display
		const failedLoad = loadFailure !== null && loadFailure === $activeNotePath;
		if (!loadedPath && !failedLoad) return;
		const origin = captureOrigin();
		const isCurrent = () => !editorDisposed && origin.path === loadedPath
			&& origin.path === $activeNotePath && origin.vault === $appConfig?.active_vault
			&& saves.isCurrentSession(origin.snapshot) && isSource === $sourceMode;

		if (isSource && !lastSourceMode) {
			clearOutline();
			// Markdown conversion is lossy; carry both selection ends by visible text.
			const selection = editor?.state.selection;
			const visibleBefore = (position: number) => editor
				? editor.state.doc.textBetween(0, position, '\n', '').replace(/\s/g, '').length : 0;
			const anchorNonWs = visibleBefore(selection?.anchor ?? 0);
			const headNonWs = selection?.head === selection?.anchor ? anchorNonWs : visibleBefore(selection?.head ?? 0);
			const docNonWs = docNonWhitespace();
			const text = stripTitleH1(editor && !failedLoad ? editorToMarkdown() : ($activeNote?.content ?? ''));
			source.load(text);
			if (failedLoad) {
				// The source view holds the real content, so the note counts as loaded again.
				loadedPath = $activeNotePath ?? '';
				loadFailure = null;
			}
			lastSourceMode = true;
			if (showOutline) updateOutline();
			const anchor = scanAlign(text, docNonWs, { stopAtNw: anchorNonWs }).srcOffset;
			const head = headNonWs === anchorNonWs ? anchor : scanAlign(text, docNonWs, { stopAtNw: headNonWs }).srcOffset;
			tick().then(() => {
				if (!isCurrent() || !source.view) return;
				if (noteSearchOpen) return;
				source.select(anchor, head, true);
				if (!isMobile) source.focus();
			});
		} else if (!isSource && lastSourceMode) {
			clearOutline();
			lastSourceMode = false;
			const selection = source.state.selection.main;
			const srcText = source.state.doc.toString();
			const restoreRichCaret = () => {
				if (!isCurrent() || !editor || noteSearchOpen) return;
				const visible = docNonWhitespace();
				const positionAt = (offset: number) => {
					const count = scanAlign(srcText, visible, { limit: offset }).nwCount;
					return Math.min(docPosForNonWsCount(editor!.state.doc, count), editor!.state.doc.content.size);
				};
				const anchor = positionAt(selection.anchor);
				const head = selection.empty ? anchor : positionAt(selection.head);
				const sel = TextSelection.between(editor.state.doc.resolve(anchor), editor.state.doc.resolve(head));
				editor.view.dispatch(editor.state.tr.setSelection(sel).scrollIntoView());
				if (!isMobile) editor.view.focus();
				requestAnimationFrame(() => { if (isCurrent()) editor?.commands.scrollIntoView(); });
			};
			if (isMobile) {
				// A source-only first load has no rich editor yet.
				if (editor) {
					ignoreNextUpdate = true;
					editor.commands.setContent(markdownToHtml(restoreTitleH1(srcText)));
					ignoreNextUpdate = false;
				} else {
					createEditor(restoreTitleH1(srcText));
				}
				updateCounts();
				if (showOutline) updateOutline();
				tick().then(restoreRichCaret);
			} else {
				// Desktop: destroy old editor (its DOM element is gone),
				// Wait for the source component to unmount before recreating the rich view.
				destroyEditor();
				const content = srcText;
				tick().then(() => {
					if (isCurrent() && editorElement && !editor) {
						createEditor(restoreTitleH1(content));
						restoreRichCaret();
						if ($editorDirty) void forceSave();
					}
				});
			}
		}
	});

	// Tauri drag-drop listener for OS file drops (browser DragEvent doesn't have files in Tauri)
	$effect(() => {
		let disposed = false;
		let unlisten: (() => void) | undefined;
		getCurrentWindow().onDragDropEvent((event) => {
			if (event.payload.type !== 'drop') return;
			const origin = captureOrigin();
			if (!canInsert(origin)) return;
			for (const filePath of event.payload.paths) {
				const name = filePath.replace(/\\/g, '/').split('/').pop() || 'file';
				const ext = name.split('.').pop()?.toLowerCase() || '';
				const kind: AssetKind = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'bmp', 'ico'].includes(ext)
					? 'image' : ext === 'pdf' ? 'pdf' : 'file';
				void readFile(filePath).then(async (data) => {
					if (disposed || !canInsert(origin)) return;
					await uploadAsset(origin, name, Array.from(data), kind);
				}).catch((e) => console.error('Failed to drop file:', e));
			}
		}).then((fn) => {
			if (disposed) fn();
			else unlisten = fn;
		}).catch((e) => console.error('Failed to listen for file drops:', e));
		return () => { disposed = true; unlisten?.(); };
	});

	// Note events update one cached title; only structural changes need a vault scan.
	let unlistenFileChange: (() => void) | null = null;
	listen<FileEvent>('file-changed', (event) => {
		handleWikiFileChange(event.payload);
	}).then(fn => {
		if (editorDisposed) fn();
		else unlistenFileChange = fn;
	}).catch(e => console.error('Failed to listen for note changes:', e));

	type PathMovedEvent = {
		vault_path: string; old_path: string; new_path: string;
		old_raw?: string | null; new_raw?: string | null; old_title?: string | null; new_title?: string | null;
	};
	let unlistenPathMoved: (() => void) | null = null;
	listen<PathMovedEvent>('vault-path-moved', ({ payload }) => {
		if (editorDisposed || externalOperationBusy || payload.vault_path !== $appConfig?.active_vault) return;
		const path = $activeNotePath;
		if (!path || (path !== payload.old_path
			&& !path.startsWith(payload.old_path + '/') && !path.startsWith(payload.old_path + '\\'))) return;
		const migrated = payload.new_path + path.slice(payload.old_path.length);
		const previous = $activeNote;
		if (!previous) return;
		const session = saves.capture();
		const oldRaw = saves.raw;
		pathMigrationPending = true;
		loadedPath = migrated;
		$activeNotePath = migrated;
		$activeNote = { ...previous, path: migrated };
		navHistory.remap(payload.old_path, payload.new_path);
		if (showOutline) updateOutline();
		void (async () => {
			try {
				await saves.idle();
				const content = await readNote(payload.vault_path, migrated);
				if (editorDisposed || !saves.isCurrentSession(session) || $activeNotePath !== migrated) return;
				const renamedRaw = typeof payload.old_raw === 'string';
				const baselineMatches = renamedRaw ? oldRaw === payload.old_raw && saves.raw === oldRaw
					&& (typeof payload.new_raw !== 'string' || payload.new_raw === content.raw)
					: saves.raw === content.raw;
				if ($editorDirty) {
					if (!baselineMatches) { flagExternalConflict(); return; }
					if (renamedRaw) saves.adoptMovedRaw(oldRaw, content.raw);
					if (typeof payload.old_title === 'string' && typeof payload.new_title === 'string'
						&& $activeNote?.meta.title === payload.old_title) {
						$activeNote = { ...$activeNote, meta: { ...$activeNote.meta, title: payload.new_title } };
					}
				} else {
					$activeNote = content;
					loadNote(migrated, content.content);
				}
			} catch (error) {
				console.error('Failed to refresh relocated note:', error);
				flagExternalConflict();
			} finally {
				pathMigrationPending = false;
				if ($editorDirty && !saveError?.conflict && $activeNotePath === migrated) {
					saves.schedule(() => { void forceSave(); });
				}
			}
		})();
	}).then(fn => {
		if (editorDisposed) fn();
		else unlistenPathMoved = fn;
	}).catch(error => console.error('Failed to listen for relocated notes:', error));

	/** Bring the wiki links of the open note in line with a rename of the note they point to. */
	function applyRenamedLinks(rename: LinkRename) {
		if ($sourceMode) {
			const view = source.view;
			if (!view) return;
			const changes = renamedLinkChanges(view.state.doc.toString(), rename);
			if (changes.length) view.dispatch({ changes });
			return;
		}
		if (!editor) return;
		const { schema, doc } = editor.state;
		const wikiLink = schema.marks.wikiLink;
		const edits: { from: number; to: number; node: any }[] = [];
		doc.descendants((node: any, pos: number) => {
			if (!node.isText) return;
			const text = node.text as string;
			const mark = wikiLink ? node.marks.find((m: any) => m.type === wikiLink) : null;
			if (mark) {
				const ref = (mark.attrs.title as string | null) || text;
				const next = renamedLink(ref, ref === text ? null : text, rename);
				if (!next) return;
				const path = String(mark.attrs.path ?? '').replace(/\\/g, '/');
				const attrs = { ...mark.attrs, title: next.ref,
					path: path === `${rename.old_ref}.md` ? `${rename.new_ref}.md` : mark.attrs.path };
				const marks = node.marks.map((m: any) => m === mark ? wikiLink.create(attrs) : m);
				edits.push({ from: pos, to: pos + node.nodeSize, node: schema.text(next.display ?? next.ref, marks) });
			} else if (!wikiLink && !node.marks.some((m: any) => m.type.name === 'code')) {
				// With wiki links switched off they are ordinary text in the note.
				for (const change of renamedLinkChanges(text, rename)) {
					edits.push({ from: pos + change.from, to: pos + change.to, node: schema.text(change.insert, node.marks) });
				}
			}
		});
		if (!edits.length) return;
		const tr = editor.state.tr.setMeta('addToHistory', false);
		for (const edit of edits.reverse()) tr.replaceWith(edit.from, edit.to, edit.node);
		editor.view.dispatch(tr);
	}

	type LinksRewrittenEvent = LinkRename & { vault_path: string; path: string; old_raw: string; new_raw: string };
	let unlistenLinksRewritten: (() => void) | null = null;
	listen<LinksRewrittenEvent>('vault-links-rewritten', ({ payload }) => {
		if (editorDisposed || payload.vault_path !== $appConfig?.active_vault
			|| payload.path !== $activeNotePath || payload.path !== loadedPath) return;
		// Without unsaved edits the note is simply reloaded when the file change is noticed.
		if (!$editorDirty) return;
		const session = saves.capture();
		void (async () => {
			await saves.idle();
			if (editorDisposed || !saves.isCurrentSession(session) || $activeNotePath !== payload.path) return;
			// The rename rewrote the version this draft is based on: take over the same link changes
			// and continue from the rewritten file, so the draft saves instead of conflicting.
			if (!saves.adoptMovedRaw(payload.old_raw, payload.new_raw)) {
				if (saves.raw !== payload.new_raw) flagExternalConflict();
				return;
			}
			applyRenamedLinks(payload);
		})();
	}).then(fn => {
		if (editorDisposed) fn();
		else unlistenLinksRewritten = fn;
	}).catch(error => console.error('Failed to listen for updated links:', error));

	let unlistenToolbarLayout: (() => void) | null = null;
	listen<ToolbarLayout | null>('toolbar-layout-changed', (event) => {
		const preference = JSON.stringify(event.payload);
		if (toolbarDraft && preference !== toolbarBase && preference !== toolbarPendingPreference) toolbarConflict = true;
		if (!toolbarDraft) closeAllDropdowns();
		appConfig.update(config => config ? { ...config, toolbar_layout: event.payload } : config);
	}).then(fn => {
		if (editorDisposed) fn();
		else unlistenToolbarLayout = fn;
	}).catch(e => console.error('Failed to listen for toolbar changes:', e));

	function handleOpenNote(event: Event) {
		const path = (event as CustomEvent<string>).detail;
		if (typeof path === 'string' && path) void navigateToWikiLink(path, '');
	}
	window.addEventListener('helixnotes-open-note', handleOpenNote);

	onDestroy(() => {
		cancelProgressiveLoad();
		void flushSave();
		closeAiMenu();
		endToolbarCustomization();
		editorDisposed = true;
		if (editorNavigationToken !== null) cancelNoteNavigation(editorNavigationToken);
		saves.startSession();
		cancelCounts();
		clearOutline();
		cancelNoteSearchWork();
		searchRevealedDetails.clear();
		if (wikiTitleRefreshTimer !== null) clearTimeout(wikiTitleRefreshTimer);
		window.removeEventListener('helixnotes-open-note', handleOpenNote);
		clearTargetReveal();
		destroyEditor();
		unlistenFileChange?.();
		unlistenPathMoved?.();
		unlistenLinksRewritten?.();
		unlistenToolbarLayout?.();
		if (infoPathCopyTimer) clearTimeout(infoPathCopyTimer);
	});
</script>

{#snippet tableSizePicker()}
	<div class="fmt-dropdown table-picker-dropdown" role="dialog" aria-label="Formatting options" use:positionFormattingPopup>
		<div class="table-picker-grid">
			{#each Array(8) as _, r}
				{#each Array(10) as _, c}
					<button
						type="button"
						class="table-picker-cell"
						class:active={r < tablePickerHover.rows && c < tablePickerHover.cols}
						aria-label={`Insert ${r + 1} by ${c + 1} table`}
						onmouseenter={() => tablePickerHover = { rows: r + 1, cols: c + 1 }}
						onclick={() => insertTable(r + 1, c + 1)}
					></button>
				{/each}
			{/each}
		</div>
		<div class="table-picker-label">
			{tablePickerHover.rows > 0 ? `${tablePickerHover.rows} x ${tablePickerHover.cols}` : 'Select size'}
		</div>
	</div>
{/snippet}

{#snippet toolbarControl(id: ToolbarButtonId)}
	{#if id === 'insert'}
		<div class="fmt-dropdown-wrap">
			<button class="fmt-btn insert-btn" onclick={(e) => toggleFormattingPicker(e, 'insert')} aria-haspopup="dialog" aria-expanded={insertDropdown || (isMobile && tablePickerOpen)} title="Insert">
				<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h14"/><path d="M12 5v14"/></svg>
			</button>
			{#if insertDropdown}
				<div class="fmt-dropdown insert-dropdown" role="dialog" aria-label="Formatting options" use:positionFormattingPopup>
					<button onclick={() => openAssetPicker('insert-image-input')}>
						<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="18" height="18" x="3" y="3" rx="2" ry="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.086-3.086a2 2 0 00-2.828 0L6 21"/></svg>
						Image
					</button>
					{#if isAndroid}
						<button onclick={() => openAssetPicker('insert-camera-input')}>
							<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14.5 4h-5L7 7H4a2 2 0 00-2 2v9a2 2 0 002 2h16a2 2 0 002-2V9a2 2 0 00-2-2h-3l-2.5-3z"/><circle cx="12" cy="13" r="3"/></svg>
							Take Photo
						</button>
					{/if}
					<button onclick={() => openAssetPicker('insert-file-input')}>
						<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 22a2 2 0 01-2-2V4a2 2 0 012-2h8a2.4 2.4 0 011.704.706l3.588 3.588A2.4 2.4 0 0120 8v12a2 2 0 01-2 2z"/><path d="M14 2v5a1 1 0 001 1h5"/></svg>
						File
					</button>
					{#if isMobile}
					<button onclick={() => { insertDropdown = false; editor?.chain().focus().toggleCodeBlock().run(); }}>
						<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m10 9-3 3 3 3"/><path d="m14 15 3-3-3-3"/><rect x="3" y="3" width="18" height="18" rx="2"/></svg>
						Code Block
					</button>
					<button onclick={() => { insertDropdown = false; editor?.chain().focus().toggleBlockquote().run(); }}>
						<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17 5H3"/><path d="M21 12H8"/><path d="M21 19H8"/><path d="M3 12v7"/></svg>
						Quote
					</button>
					<button onclick={(e) => toggleFormattingPicker(e, 'table')}>
						<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3v18"/><rect width="18" height="18" x="3" y="3" rx="2"/><path d="M3 9h18"/><path d="M3 15h18"/></svg>
						Table
					</button>
					<button onclick={() => { insertDropdown = false; insertDetails(); }}>
						<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="13" height="7" x="8" y="3" rx="1"/><path d="m2 9 3 3-3 3"/><rect width="13" height="7" x="8" y="14" rx="1"/></svg>
						Collapsible Section
					</button>
					<button onclick={() => { insertDropdown = false; insertCallout('note'); }}>
						<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="5" width="18" height="14" rx="2"/><line x1="7" y1="5" x2="7" y2="19"/></svg>
						Callout
					</button>
					{/if}
					<button onclick={() => { insertDropdown = false; editor?.chain().focus().setHorizontalRule().run(); }}>
						<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M5 12h14"/></svg>
						Horizontal Rule
					</button>
					<button onclick={() => { insertDropdown = false; openSecretInsert(); }}>
						<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="11" width="18" height="10" rx="2"/><path d="M7 11V7a5 5 0 0110 0v4"/></svg>
						Secret
					</button>
					{#if !isMobile}
					<button onclick={() => { insertDropdown = false; insertTimestamp('datetime'); }}>
						<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 7.5V6a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2h6"/><line x1="3" y1="10" x2="21" y2="10"/><line x1="8" y1="2" x2="8" y2="6"/><line x1="16" y1="2" x2="16" y2="6"/><circle cx="18" cy="17" r="4"/><path d="M18 15.5v1.5l1 1"/></svg>
						Date &amp; Time
					</button>
					{/if}
				</div>
			{/if}
			{#if isMobile && tablePickerOpen}
				{@render tableSizePicker()}
			{/if}
		</div>
	{:else if id === 'heading'}
		<div class="fmt-dropdown-wrap">
			<button class="fmt-btn" class:active={isEditorActive('heading')} onclick={(e) => toggleFormattingPicker(e, 'heading')} aria-haspopup="dialog" aria-expanded={headingDropdown} title="Heading">
				<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 12h12"/><path d="M6 20V4"/><path d="M18 20V4"/></svg>
			</button>
			{#if headingDropdown}
				<div class="fmt-dropdown" role="dialog" aria-label="Formatting options" use:positionFormattingPopup>
					<button class:active={isEditorActive('heading', { level: 1 })} onclick={() => { editor?.chain().focus().toggleHeading({ level: 1 }).run(); headingDropdown = false; }}>Heading 1</button>
					<button class:active={isEditorActive('heading', { level: 2 })} onclick={() => { editor?.chain().focus().toggleHeading({ level: 2 }).run(); headingDropdown = false; }}>Heading 2</button>
					<button class:active={isEditorActive('heading', { level: 3 })} onclick={() => { editor?.chain().focus().toggleHeading({ level: 3 }).run(); headingDropdown = false; }}>Heading 3</button>
					{#if !isMobile}<button class:active={isEditorActive('heading', { level: 4 })} onclick={() => { editor?.chain().focus().toggleHeading({ level: 4 }).run(); headingDropdown = false; }}>Heading 4</button>{/if}
					<button class:active={isEditorActive('paragraph')} onclick={() => { editor?.chain().focus().setParagraph().run(); headingDropdown = false; }}>Paragraph</button>
				</div>
			{/if}
		</div>
	{:else if id === 'bold'}
		<button class="fmt-btn" class:active={isEditorActive('bold')} onclick={() => editor?.chain().focus().toggleBold().run()} title={isMobile ? "Bold" : `Bold (${modKey}+B)`}>
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 12h9a4 4 0 010 8H7a1 1 0 01-1-1V5a1 1 0 011-1h7a4 4 0 010 8"/></svg>
		</button>
	{:else if id === 'italic'}
		<button class="fmt-btn" class:active={isEditorActive('italic')} onclick={() => editor?.chain().focus().toggleItalic().run()} title={isMobile ? "Italic" : `Italic (${modKey}+I)`}>
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="19" x2="10" y1="4" y2="4"/><line x1="14" x2="5" y1="20" y2="20"/><line x1="15" x2="9" y1="4" y2="20"/></svg>
		</button>
	{:else if id === 'underline'}
		<button class="fmt-btn" class:active={isEditorActive('underline')} onclick={() => editor?.chain().focus().toggleUnderline().run()} title={isMobile ? "Underline" : `Underline (${modKey}+U)`}>
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 4v6a6 6 0 0012 0V4"/><line x1="4" x2="20" y1="20" y2="20"/></svg>
		</button>
	{:else if id === 'strike'}
		<button class="fmt-btn" class:active={isEditorActive('strike')} onclick={() => editor?.chain().focus().toggleStrike().run()} title={isMobile ? "Strikethrough" : `Strikethrough (${modKey}+Shift+X)`}>
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M16 4H9a3 3 0 00-2.83 4"/><path d="M14 12a4 4 0 010 8H6"/><line x1="4" x2="20" y1="12" y2="12"/></svg>
		</button>
	{:else if id === 'text-color'}
		<div class="fmt-dropdown-wrap">
			<button class="fmt-btn" onclick={(e) => toggleFormattingPicker(e, 'text-color')} aria-haspopup="dialog" aria-expanded={colorDropdown} title="Text Color">
				<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 20h16"/><path d="m6 16 6-12 6 12"/><path d="M8 12h8"/></svg>
				<span class="color-indicator" style="background: {getEditorAttributes('textStyle').color || 'var(--accent)'}"></span>
			</button>
			{#if colorDropdown}
				<div class="fmt-dropdown color-grid-dropdown" role="dialog" aria-label="Formatting options" use:positionFormattingPopup>
					{#each textColors as color}
						<button class="color-swatch" title={color.name} onclick={() => setTextColor(color.value)} style="background: {color.value || 'var(--text-primary)'}">
							{#if (color.value === '' && !getEditorAttributes('textStyle').color) || getEditorAttributes('textStyle').color === color.value}
								<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="3"><polyline points="20 6 9 17 4 12"/></svg>
							{/if}
						</button>
					{/each}
				</div>
			{/if}
		</div>
	{:else if id === 'link'}
		<button class="fmt-btn" class:active={isEditorActive('link')} onclick={addLinkFromToolbar} title={isMobile ? "Link" : `Link (${modKey}+K)`}>
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10 13a5 5 0 007.54.54l3-3a5 5 0 00-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 00-7.54-.54l-3 3a5 5 0 007.07 7.07l1.71-1.71"/></svg>
		</button>
	{:else if id === 'bullet-list'}
		<button class="fmt-btn" class:active={isEditorActive('bulletList')} onclick={toggleBulletList} title={isMobile ? "Bullet List" : `Bullet List (${modKey}+Shift+8)`}>
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 5h.01"/><path d="M3 12h.01"/><path d="M3 19h.01"/><path d="M8 5h13"/><path d="M8 12h13"/><path d="M8 19h13"/></svg>
		</button>
	{:else if id === 'ordered-list'}
		<button class="fmt-btn" class:active={isEditorActive('orderedList')} onclick={() => editor?.chain().focus().toggleOrderedList().run()} title={isMobile ? "Numbered List" : `Ordered List (${modKey}+Shift+7)`}>
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11 5h10"/><path d="M11 12h10"/><path d="M11 19h10"/><path d="M4 4h1v5"/><path d="M4 9h2"/><path d="M6.5 20H3.4c0-1 2.6-1.925 2.6-3.5a1.5 1.5 0 00-2.6-1.02"/></svg>
		</button>
	{:else if id === 'task-list'}
		<button class="fmt-btn" class:active={isEditorActive('taskList')} onclick={toggleTaskList} title={isMobile ? "Task List" : `Task List (${modKey}+Shift+9)`}>
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M13 5h8"/><path d="M13 12h8"/><path d="M13 19h8"/><path d="m3 17 2 2 4-4"/><path d="m3 7 2 2 4-4"/></svg>
		</button>
	{:else if id === 'undo'}
		<button class="fmt-btn" onclick={() => editor?.chain().focus().undo().run()} title={isMobile ? "Undo" : `Undo (${modKey}+Z)`}>
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 015.5 5.5 5.5 5.5 0 01-5.5 5.5H11"/></svg>
		</button>
	{:else if id === 'redo'}
		<button class="fmt-btn" onclick={() => editor?.chain().focus().redo().run()} title={isMobile ? "Redo" : `Redo (${modKey}+Shift+Z)`}>
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m15 14 5-5-5-5"/><path d="M20 9H9.5A5.5 5.5 0 004 14.5 5.5 5.5 0 009.5 20H13"/></svg>
		</button>
	{:else if id === 'inline-code'}
		<button class="fmt-btn" class:active={isEditorActive('code')} onclick={() => editor?.chain().focus().toggleCode().run()} title={`Inline Code (${modKey}+E)`}>
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m16 18 6-6-6-6"/><path d="m8 6-6 6 6 6"/></svg>
		</button>
	{:else if id === 'code-block'}
		<button class="fmt-btn" class:active={isEditorActive('codeBlock')} onclick={() => editor?.chain().focus().toggleCodeBlock().run()} title={`Code Block (${modKey}+Alt+C)`}>
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m10 9-3 3 3 3"/><path d="m14 15 3-3-3-3"/><rect x="3" y="3" width="18" height="18" rx="2"/></svg>
		</button>
	{:else if id === 'quote'}
		<button class="fmt-btn" class:active={isEditorActive('blockquote')} onclick={() => editor?.chain().focus().toggleBlockquote().run()} title={`Quote (${modKey}+Shift+B)`}>
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17 5H3"/><path d="M21 12H8"/><path d="M21 19H8"/><path d="M3 12v7"/></svg>
		</button>
	{:else if id === 'details'}
		<button class="fmt-btn" class:active={isEditorActive('details')} onclick={() => insertDetails()} title={`Collapsible Section (${modKey}+.)`}>
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="13" height="7" x="8" y="3" rx="1"/><path d="m2 9 3 3-3 3"/><rect width="13" height="7" x="8" y="14" rx="1"/></svg>
		</button>
	{:else if id === 'callout'}
		<button class="fmt-btn" class:active={isEditorActive('callout')} onclick={() => insertCallout('note')} title="Callout">
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="5" width="18" height="14" rx="2"/><line x1="7" y1="5" x2="7" y2="19"/></svg>
		</button>
	{:else if id === 'table'}
		<div class="fmt-dropdown-wrap">
			<button class="fmt-btn" onclick={(e) => toggleFormattingPicker(e, 'table')} aria-haspopup="dialog" aria-expanded={tablePickerOpen} title="Insert Table">
				<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3v18"/><rect width="18" height="18" x="3" y="3" rx="2"/><path d="M3 9h18"/><path d="M3 15h18"/></svg>
			</button>
			{#if tablePickerOpen}
				{@render tableSizePicker()}
			{/if}
		</div>
	{:else if id === 'horizontal-rule'}
		<button class="fmt-btn" onclick={() => editor?.chain().focus().setHorizontalRule().run()} title="Horizontal Rule">
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M5 12h14"/></svg>
		</button>
	{:else if id === 'highlight'}
		{#if isMobile}
			<button class="fmt-btn" class:active={isEditorActive('highlight')} onclick={() => editor?.chain().focus().toggleHighlight({ color: highlightColors[0].value }).run()} title="Highlight">
				<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 11-6 6v3h9l3-3"/><path d="m22 12-4.6 4.6a2 2 0 01-2.8 0l-5.2-5.2a2 2 0 010-2.8L14 4"/></svg>
			</button>
		{:else}
			<div class="fmt-dropdown-wrap">
				<button class="fmt-btn" class:active={isEditorActive('highlight')} onclick={(e) => toggleFormattingPicker(e, 'highlight')} aria-haspopup="dialog" aria-expanded={highlightDropdown} title={isMobile ? "Highlight" : `Highlight (${modKey}+Shift+H)`}>
					<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 11-6 6v3h9l3-3"/><path d="m22 12-4.6 4.6a2 2 0 01-2.8 0l-5.2-5.2a2 2 0 010-2.8L14 4"/></svg>
					<span class="color-indicator" style="background: {getEditorAttributes('highlight').color || 'var(--accent)'}"></span>
				</button>
				{#if highlightDropdown}
					<div class="fmt-dropdown color-grid-dropdown" role="dialog" aria-label="Formatting options" use:positionFormattingPopup>
						{#each highlightColors as color}
							<button class="color-swatch" title={color.name} onclick={() => setHighlightColor(color.value)} style="background: {color.swatch}">
								{#if isEditorActive('highlight', { color: color.value })}
									<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="3"><polyline points="20 6 9 17 4 12"/></svg>
								{/if}
							</button>
						{/each}
						<button class="color-swatch" title="Remove highlight" onclick={() => setHighlightColor('')} style="background: var(--bg-tertiary);">
							{#if !isEditorActive('highlight')}
								<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="var(--text-primary)" stroke-width="3"><polyline points="20 6 9 17 4 12"/></svg>
							{:else}
								<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="var(--text-tertiary)" stroke-width="2"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
							{/if}
						</button>
					</div>
				{/if}
			</div>
		{/if}
	{:else if id === 'subscript'}
		<button class="fmt-btn" class:active={isEditorActive('subscript')} onclick={() => editor?.chain().focus().toggleSubscript().run()} title="Subscript">
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m4 5 8 8"/><path d="m12 5-8 8"/><path d="M20 19h-4c0-1.5.44-2 1.5-2.5S20 15.33 20 14c0-.47-.17-.93-.48-1.29a2.11 2.11 0 00-2.62-.44c-.42.24-.74.62-.9 1.07"/></svg>
		</button>
	{:else if id === 'superscript'}
		<button class="fmt-btn" class:active={isEditorActive('superscript')} onclick={() => editor?.chain().focus().toggleSuperscript().run()} title="Superscript">
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m4 19 8-8"/><path d="m12 19-8-8"/><path d="M20 12h-4c0-1.5.442-2 1.5-2.5S20 8.334 20 7.002c0-.472-.17-.93-.484-1.29a2.105 2.105 0 00-2.617-.436c-.42.239-.738.614-.899 1.06"/></svg>
		</button>
	{:else if id === 'alignment'}
		<div class="fmt-dropdown-wrap">
			<button class="fmt-btn" onclick={(e) => toggleFormattingPicker(e, 'alignment')} aria-haspopup="dialog" aria-expanded={alignDropdown} title="Text Alignment">
				{#if isEditorActive({ textAlign: 'center' })}
					<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 5H3"/><path d="M17 12H7"/><path d="M19 19H5"/></svg>
				{:else if isEditorActive({ textAlign: 'right' })}
					<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 5H3"/><path d="M21 12H9"/><path d="M21 19H7"/></svg>
				{:else if isEditorActive({ textAlign: 'justify' })}
					<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 5h18"/><path d="M3 12h18"/><path d="M3 19h18"/></svg>
				{:else}
					<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 5H3"/><path d="M15 12H3"/><path d="M17 19H3"/></svg>
				{/if}
			</button>
			{#if alignDropdown}
				<div class="fmt-dropdown align-dropdown" role="dialog" aria-label="Formatting options" use:positionFormattingPopup>
					<button class:active={isEditorActive({ textAlign: 'left' })} onclick={() => { editor?.chain().focus().setTextAlign('left').run(); alignDropdown = false; }}>
						<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 5H3"/><path d="M15 12H3"/><path d="M17 19H3"/></svg>
						Left
					</button>
					<button class:active={isEditorActive({ textAlign: 'center' })} onclick={() => { editor?.chain().focus().setTextAlign('center').run(); alignDropdown = false; }}>
						<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 5H3"/><path d="M17 12H7"/><path d="M19 19H5"/></svg>
						Center
					</button>
					<button class:active={isEditorActive({ textAlign: 'right' })} onclick={() => { editor?.chain().focus().setTextAlign('right').run(); alignDropdown = false; }}>
						<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 5H3"/><path d="M21 12H9"/><path d="M21 19H7"/></svg>
						Right
					</button>
					<button class:active={isEditorActive({ textAlign: 'justify' })} onclick={() => { editor?.chain().focus().setTextAlign('justify').run(); alignDropdown = false; }}>
						<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 5h18"/><path d="M3 12h18"/><path d="M3 19h18"/></svg>
						Justify
					</button>
				</div>
			{/if}
		</div>
	{:else if id === 'indent'}
		<button class="fmt-btn" onclick={() => {
			if (!editor) return;
			// Try list indent first - run() returns true if it succeeded
			const sank = editor.chain().focus().sinkListItem('listItem').run();
			if (!sank) {
				const sankTask = editor.chain().focus().sinkListItem('taskItem').run();
				if (!sankTask && editor.state.selection.empty) {
					editor.chain().focus().insertContent('\t').run();
				}
			}
		}} title={isMobile ? "Indent" : "Indent (Tab)"}>
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="3" y1="4" x2="21" y2="4"/><line x1="11" y1="9" x2="21" y2="9"/><line x1="11" y1="14" x2="21" y2="14"/><line x1="3" y1="19" x2="21" y2="19"/><polyline points="3 9 7 11.5 3 14"/></svg>
		</button>
	{:else if id === 'outdent'}
		<button class="fmt-btn" onclick={() => {
			if (!editor) return;
			const lifted = editor.chain().focus().liftListItem('listItem').run();
			if (!lifted) {
				const liftedTask = editor.chain().focus().liftListItem('taskItem').run();
				if (!liftedTask && editor.state.selection.empty) {
					// Remove leading tab/spaces from current line
					const { from } = editor.state.selection;
					const pos = editor.state.doc.resolve(from);
					const lineStart = pos.start(pos.depth);
					const lineText = editor.state.doc.textBetween(lineStart, pos.end(pos.depth));
					if (lineText.startsWith('\t')) {
						editor.chain().focus().command(({ tr }) => {
							tr.delete(lineStart, lineStart + 1);
							return true;
						}).run();
					} else if (lineText.startsWith('    ')) {
						editor.chain().focus().command(({ tr }) => {
							tr.delete(lineStart, lineStart + 4);
							return true;
						}).run();
					} else if (lineText.startsWith('  ')) {
						editor.chain().focus().command(({ tr }) => {
							tr.delete(lineStart, lineStart + 2);
							return true;
						}).run();
					}
				}
			}
		}} title={isMobile ? "Outdent" : "Outdent (Shift+Tab)"}>
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="3" y1="4" x2="21" y2="4"/><line x1="11" y1="9" x2="21" y2="9"/><line x1="11" y1="14" x2="21" y2="14"/><line x1="3" y1="19" x2="21" y2="19"/><polyline points="7 9 3 11.5 7 14"/></svg>
		</button>
	{:else if id === 'date-time'}
		<button class="fmt-btn" onclick={() => insertTimestamp('datetime')} title="Insert date and time">
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 7.5V6a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2h6"/><line x1="3" y1="10" x2="21" y2="10"/><line x1="8" y1="2" x2="8" y2="6"/><line x1="16" y1="2" x2="16" y2="6"/><circle cx="18" cy="17" r="4"/><path d="M18 15.5v1.5l1 1"/></svg>
		</button>
	{:else if id === 'ai-actions'}
		<button class="fmt-btn" onclick={openAiMenu} title="AI Actions">
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
				<path d="M12 8V4l-2-2"/><rect x="4" y="8" width="16" height="12" rx="2"/><path d="M2 14h2"/><path d="M20 14h2"/><path d="M9 13v2"/><path d="M15 13v2"/>
			</svg>
		</button>
	{/if}
{/snippet}

{#snippet editableToolbarControl(id: ToolbarButtonId)}
	<div
		class="fmt-control"
		class:selected={toolbarSelected === id}
		class:dragging={toolbarDraggedId === id}
		data-toolbar-id={id}
		use:customizeToolbarItem={{ id, selected: toolbarSelected === id }}
	>
		{@render toolbarControl(id)}
	</div>
{/snippet}



<svelte:window onkeydown={handleNoteSearchKey} />


<div class="editor-container" class:mobile={isMobile} inert={externalOperationBusy}>
	{#if !$activeNote}
		<div class="empty-editor">
			<div class="empty-icon">
				<svg width="48" height="48" viewBox="0 0 48 48" fill="none">
					<rect x="8" y="6" width="32" height="36" rx="4" stroke="var(--text-tertiary)" stroke-width="2" fill="none" />
					<path d="M16 16h16M16 22h12M16 28h16M16 34h8" stroke="var(--text-tertiary)" stroke-width="1.5" stroke-linecap="round" />
				</svg>
			</div>
			<p>Select a note or create a new one</p>
			<div class="shortcuts-hint">
				<span><kbd>{modKey}</kbd>+<kbd>N</kbd> New note</span>
				<span><kbd>{modKey}</kbd>+<kbd>P</kbd> Quick open</span>
			</div>
		</div>
	{:else}
		{#if $viewerNote}
			<div class="viewer-banner">
				<svg class="viewer-banner-icon" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/><circle cx="12" cy="12" r="3"/>
				</svg>
				<span class="viewer-banner-label">Read-only viewer</span>
				<code class="viewer-banner-path" title={$viewerNote.path}>{$viewerNote.path}</code>
				<div class="viewer-banner-actions">
					<button type="button" class="viewer-banner-btn primary" onclick={() => (viewerImportPickerOpen = true)} disabled={viewerImportBusy || !$appConfig?.active_vault}>Import to vault…</button>
					<button type="button" class="viewer-banner-btn" onclick={closeViewer}>Close</button>
				</div>
				{#if viewerToast}
					<span class="viewer-banner-toast">{viewerToast}</span>
				{/if}
			</div>
		{/if}
		{#if !$viewerNote}
		<div class="editor-toolbar" class:mobile={isMobile}>
			<div class="editor-title" class:search-target-meta={searchTitleReveal}>
				<input
					bind:this={titleInput}
					type="text"
					readonly={$readOnly || externalOperationBusy}
					value={$activeNote.meta.title}
					oninput={(e) => {
						if (!$activeNote) return;
						const title = e.currentTarget.value;
						if ($activeNote.meta.title === title) return;
						syncVisibleTitleHeading($activeNote.meta.title, title);
						$activeNote.meta.title = title;
						if (hiddenTitleHeading) hiddenTitleHeading = { ...hiddenTitleHeading, title };
						markEdited();
					}}
					onkeydown={(e) => {
						if (e.key === 'Tab') {
							e.preventDefault();
							if ($sourceMode) { source.select(0); source.focus(); }
							else editor?.commands.focus('start');
						}
						if (e.key === 'Enter') {
							e.preventDefault();
							if ($sourceMode) { source.select(0); source.focus(); }
							else editor?.commands.focus('start');
						}
					}}
					onchange={async (e) => {
						if ($activeNote && $activeNotePath) {
							const newTitle = (e.target as HTMLInputElement).value.trim();
							if (!newTitle) return;
							const oldPath = $activeNotePath;
							if ($activeNote.meta.title !== newTitle) {
								syncVisibleTitleHeading($activeNote.meta.title, newTitle);
								$activeNote.meta.title = newTitle;
								if (hiddenTitleHeading) hiddenTitleHeading = { ...hiddenTitleHeading, title: newTitle };
								markEdited();
							}
							const origin = captureOrigin();
							if (!origin.vault) return;
							if (!(await forceSave()) || !isOriginCurrent(origin, true)) return;
							// Rename file on disk if filename doesn't match the new title
							const filename = oldPath.split('/').pop() ?? '';
							const stem = filename.replace(/\.md$/, '');
							if (stem !== newTitle) {
								renamingNote = origin.snapshot;
								try {
									const newPath = await renameNote(origin.vault, oldPath, newTitle);
									if (isOriginCurrent(origin)) {
										loadedPath = newPath;
										$activeNotePath = newPath;
										if (showOutline) updateOutline();
									}
									if (origin.vault !== $appConfig?.active_vault) return;
									notes.update(list => list.map(n =>
										n.path === oldPath
											? { ...n, path: newPath, relative_path: n.relative_path.replace(/[^/]+$/, newTitle + '.md'), meta: { ...n.meta, title: newTitle } }
											: n
									));
									// Refresh wiki-link titles cache so links resolve to renamed note
									refreshWikiLinkTitles();
								} catch (err) {
									console.error('Failed to rename note file:', err);
									if (!isOriginCurrent(origin)) return;
									notes.update(list => list.map(n =>
										n.path === oldPath ? { ...n, meta: { ...n.meta, title: newTitle } } : n
									));
								} finally {
									if (renamingNote === origin.snapshot) renamingNote = null;
									if (origin.vault === $appConfig?.active_vault && $editorDirty) void forceSave();
								}
							} else {
								notes.update(list => list.map(n =>
									n.path === oldPath ? { ...n, meta: { ...n.meta, title: newTitle } } : n
								));
							}
						}
					}}
				/>
			</div>
			{#if !isMobile}
			<div class="toolbar-actions">
				{#if $canGoBack || $canGoForward}
				<div class="nav-history-btns">
					<button class="nav-history-btn" disabled={!$canGoBack} onclick={() => editorNavigateHistory(-1)} title="Back (Alt+←)">
						<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><polyline points="15 18 9 12 15 6"/></svg>
					</button>
					<button class="nav-history-btn" disabled={!$canGoForward} onclick={() => editorNavigateHistory(1)} title="Forward (Alt+→)">
						<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><polyline points="9 18 15 12 9 6"/></svg>
					</button>
				</div>
				{/if}
				{#if $editorDirty}
					<span class="save-indicator">Unsaved</span>
				{/if}
				{#if $readOnly}
					<span class="readonly-indicator">View Mode</span>
				{/if}
				<button
					class="icon-btn"
					class:active={noteSearchOpen}
					onclick={() => noteSearchOpen ? closeNoteSearch() : openNoteSearch()}
					title={`Find in note (${modKey}+F)`}
				>
					<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						<circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>
					</svg>
				</button>

				<button
					class="icon-btn"
					class:active={!!tagMenu}
					disabled={trashPreview}
					onclick={toggleTagMenu}
					title="Edit tags"
				>
					<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						<path d="M20.59 13.41l-7.17 7.17a2 2 0 0 1-2.83 0L2 12V2h10l8.59 8.59a2 2 0 0 1 0 2.82z"/>
						<line x1="7" y1="7" x2="7.01" y2="7"/>
					</svg>
				</button>
				<button
					class="icon-btn"
					class:active={$activeNote?.meta.pinned}
					disabled={trashPreview}
					onclick={() => {
						if ($activeNote) {
							$activeNote.meta.pinned = !$activeNote.meta.pinned;
							markEdited();
						}
					}}
					title={$activeNote?.meta.pinned ? 'Unpin note' : 'Pin note'}
				>
					<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						<path d="M12 17v5"/>
						<path d="M9 2h6l-1 7h4l-2 4H8l-2-4h4L9 2z"/>
					</svg>
				</button>
				<button
					class="icon-btn"
					class:active={isQuickAccess}
					onclick={async () => {
						if (!noteRelativePath) return;
						const origin = captureOrigin();
						const relativePath = noteRelativePath;
						if (!origin.vault) return;
						try {
							if (isQuickAccess) {
								await removeQuickAccess(origin.vault, relativePath);
							} else {
								await addQuickAccess(origin.vault, relativePath);
							}
							if (!isOriginCurrent(origin)) return;
							const qa = await getQuickAccess(origin.vault);
							if (isOriginCurrent(origin)) $quickAccessPaths = qa.map(n => n.relative_path);
						} catch (e) {
							console.error('Quick access toggle failed:', e);
						}
					}}
					title={isQuickAccess ? 'Remove from Quick Access' : 'Add to Quick Access'}
				>
					<svg width="16" height="16" viewBox="0 0 24 24" fill={isQuickAccess ? 'currentColor' : 'none'} stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						<polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" />
					</svg>
				</button>
				<button
					class="icon-btn"
					class:active={showOutline && !showHistory}
					onclick={toggleOutlinePanel}
					title="Outline"
					aria-label="Outline"
					aria-expanded={showOutline && !showHistory}
					aria-controls={outlinePanelId}
				>
					<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						<line x1="4" y1="6" x2="20" y2="6"/><line x1="8" y1="12" x2="20" y2="12"/><line x1="8" y1="18" x2="20" y2="18"/><circle cx="4" cy="12" r="1" fill="currentColor"/><circle cx="4" cy="18" r="1" fill="currentColor"/>
					</svg>
				</button>
				<button
					class="icon-btn"
					class:active={showHistory}
					onclick={toggleHistory}
					title="Version history"
				>
					<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						<circle cx="12" cy="12" r="10"/>
						<polyline points="12 6 12 12 16 14"/>
					</svg>
				</button>
				<button
					class="icon-btn"
					class:active={showInfo}
					onclick={toggleInfo}
					title="Note info"
				>
					<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						<circle cx="12" cy="12" r="10"/>
						<line x1="12" y1="8" x2="12" y2="8" stroke-width="3" stroke-linecap="round"/>
						<line x1="12" y1="12" x2="12" y2="16"/>
					</svg>
				</button>
				{#if $appConfig?.enable_wiki_links}
				<button
					class="icon-btn"
					class:active={showGraph}
					onclick={() => showGraph = !showGraph}
					title="Graph View"
				>
					<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						<circle cx="6" cy="6" r="3"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="18" r="3"/>
						<line x1="8.5" y1="7.5" x2="15.5" y2="16.5"/><line x1="15.5" y1="7.5" x2="8.5" y2="16.5"/>
					</svg>
				</button>
				{/if}
				{#if $appConfig?.ai_provider}
				<button
					class="icon-btn"
					onclick={openAiMenu}
					title="AI Actions"
				>
					<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						<path d="M12 8V4l-2-2"/><rect x="4" y="8" width="16" height="12" rx="2"/><path d="M2 14h2"/><path d="M20 14h2"/><path d="M9 13v2"/><path d="M15 13v2"/>
					</svg>
				</button>
				{/if}
				<button
					class="icon-btn"
					class:active={$sourceMode}
					onclick={() => ($sourceMode = !$sourceMode)}
					title="Toggle Markdown Editor"
				>
					<svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor">
						<path d="M5.854 4.854a.5.5 0 10-.708-.708l-3.5 3.5a.5.5 0 000 .708l3.5 3.5a.5.5 0 00.708-.708L2.707 8l3.147-3.146zm4.292 0a.5.5 0 01.708-.708l3.5 3.5a.5.5 0 010 .708l-3.5 3.5a.5.5 0 01-.708-.708L13.293 8l-3.147-3.146z" />
					</svg>
				</button>
			</div>
			{/if}
		</div>
		{/if}
		{#if externalOperationBusy}
			<div class="save-error" role="status">Updating note location or task… Editing will resume shortly.</div>
		{/if}
		{#if saveError && $editorDirty && !$viewerNote && saveError.path === $activeNotePath && saveError.vault === $appConfig?.active_vault}
			<div class="save-error" role="alert">
				<span><strong>Save failed.</strong> {saveError.message}</span>
				{#if saveError.conflict}
					<button type="button" onclick={saveConflictCopy} disabled={retryingSave} aria-label="Save draft as a copy">Save a copy</button>
					<button type="button" onclick={reloadConflictedNote} disabled={retryingSave} aria-label="Discard draft and reload note">Reload from disk</button>
				{:else}
					<button type="button" onclick={retrySave} disabled={retryingSave} aria-label="Retry saving note">
						{retryingSave ? 'Saving…' : 'Retry'}
					</button>
				{/if}
			</div>
		{/if}
		{#if loadFailure && loadFailure === $activeNotePath && !$sourceMode}
			<div class="save-error" role="alert">
				<span><strong>This note could not be opened in the editor.</strong> It is shown empty and nothing will be saved from here; the file on disk is unchanged. Switch to source mode to view or edit it.</span>
			</div>
		{/if}
		{#if searchRevealNotice}
			<div class="search-target-notice" role="status">{searchRevealNotice}</div>
		{/if}

		{#if !$focusMode}
		<div class="note-meta-bar">
			<span class="note-folder" class:unfiled={!noteFolder}>
				<svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>
				{#if noteFolder}
					{#each noteFolder.split('/') as segment, i}
						{#if i > 0}<span class="path-sep">›</span>{/if}{segment}
					{/each}
				{:else}
					Unfiled Notes
				{/if}
			</span>
			<span class="meta-divider">·</span>
			{#if onShowTag && $activeNote.meta.tags?.length > 0}
				<!-- Each tag leads to its notes; the tag button in the header edits them. -->
				<span class="note-tags-trigger" class:search-target-meta={searchTagsReveal}>
					{#each $activeNote.meta.tags as tag, i (tagIterationKey(tag, i))}
						<button type="button" class="note-tag-link" onclick={() => onShowTag(tag)} title={`Show notes tagged ${tag}`}><TagLabel name={tag} size={11} tone="muted" /></button>
					{/each}
				</span>
			{:else}
				<button class="note-tags-trigger" class:search-target-meta={searchTagsReveal} onclick={toggleTagMenu} title="Edit tags">
					{#if $activeNote.meta.tags?.length > 0}
						{#each $activeNote.meta.tags as tag, i (tagIterationKey(tag, i))}<TagLabel name={tag} size={11} tone="muted" />{/each}
					{:else}
						<span class="note-tags-add">+ Tags</span>
					{/if}
				</button>
			{/if}
		</div>
		{/if}

		<div class="editor-body-wrapper">
			{#if noteSearchOpen}
				<div class="note-search-bar">
					<button type="button" class="note-search-btn note-replace-toggle" class:open={noteReplaceOpen} aria-label="Replace" aria-expanded={noteReplaceOpen} title={`Replace (${bindingToKeys($keybindings['replace-in-note']).join('+')})`} onmousedown={(e) => e.preventDefault()} onclick={() => { noteReplaceOpen = !noteReplaceOpen; }}>
						<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="9 6 15 12 9 18"/></svg>
					</button>
					<input
						bind:this={noteSearchInput}
						type="text"
						class="note-search-input"
						placeholder="Find in note..."
						aria-label="Find in note"
						bind:value={noteSearchQuery}
						onkeydown={(e) => {
							if (e.isComposing) return;
							if (e.key === 'Enter') { e.preventDefault(); e.shiftKey ? noteSearchPrev() : noteSearchNext(); }
							if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); closeNoteSearch(); }
						}}
					/>
					<button type="button" class="note-search-btn note-search-option" aria-label="Match case" aria-pressed={noteSearchMatchCase} title="Match case" onmousedown={(e) => e.preventDefault()} onclick={() => { noteSearchMatchCase = !noteSearchMatchCase; noteSearchInput?.focus({ preventScroll: true }); }}>Aa</button>
					<button type="button" class="note-search-btn note-search-option whole-word" aria-label="Whole word" aria-pressed={noteSearchWholeWord} title="Whole word" onmousedown={(e) => e.preventDefault()} onclick={() => { noteSearchWholeWord = !noteSearchWholeWord; noteSearchInput?.focus({ preventScroll: true }); }}>ab</button>
					<span class="note-search-count" aria-live="polite" aria-atomic="true">
						{#if noteSearchPending}
							Searching…
						{:else if noteSearchQuery && noteSearchResults.length > 0}
							{noteSearchIndex + 1} / {noteSearchResults.length}
						{:else if noteSearchQuery}
							No results
						{/if}
					</span>
					<button class="note-search-btn" onclick={noteSearchPrev} onmousedown={(e) => e.preventDefault()} disabled={!noteSearchPending && !noteSearchResults.length} aria-label="Previous match" title="Previous (Shift+Enter)">
						<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="18 15 12 9 6 15"/></svg>
					</button>
					<button class="note-search-btn" onclick={noteSearchNext} onmousedown={(e) => e.preventDefault()} disabled={!noteSearchPending && !noteSearchResults.length} aria-label="Next match" title="Next (Enter)">
						<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9"/></svg>
					</button>
					<button class="note-search-btn" onclick={() => closeNoteSearch()} aria-label="Close find" title="Close (Esc)">
						<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
					</button>
				</div>
				{#if noteReplaceOpen}
					{@const replaceDisabled = (!noteSearchPending && !noteSearchResults.length) || $readOnly || !!$viewerNote || trashPreview || showHistory}
					<div class="note-search-bar note-replace-bar">
						<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="opacity:0.5;flex-shrink:0"><path d="M12 20h9"/><path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4Z"/></svg>
						<input
							bind:this={noteReplaceInput}
							type="text"
							class="note-search-input"
							placeholder="Replace with..."
							aria-label="Replace with"
							bind:value={noteReplaceValue}
							onkeydown={(e) => {
								if (e.isComposing) return;
								if (e.key === 'Enter') { e.preventDefault(); (e.ctrlKey || e.metaKey) ? replaceAllNoteMatches() : replaceNoteMatch(); }
								if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); closeNoteSearch(); }
							}}
						/>
						<button type="button" class="note-search-btn note-replace-btn" onclick={replaceNoteMatch} onmousedown={(e) => e.preventDefault()} disabled={replaceDisabled} title="Replace (Enter)">Replace</button>
						<button type="button" class="note-search-btn note-replace-btn" onclick={replaceAllNoteMatches} onmousedown={(e) => e.preventDefault()} disabled={replaceDisabled} title={`Replace all (${modKey}+Enter)`}>All</button>
					</div>
				{/if}
			{/if}
			<div class="editor-body-row" class:reviewing-history={showHistory}>
			<div class="editor-body" class:source-body={$sourceMode} class:history-hidden={showHistory} onscroll={scheduleOutlineActive}>
				{#if isMobile}
					<SourceEditor controller={source} readonly={showHistory || $readOnly || !!$viewerNote || externalOperationBusy} visible={$sourceMode} mobile />
					<!-- svelte-ignore a11y_no_static_element_interactions -->
					<div class="tiptap-wrapper" class:large-doc={isLargeDoc} style={$sourceMode ? 'display:none' : ''} spellcheck="false" bind:this={editorElement} onclick={(e) => { closeLinkContextMenu(); handleEditorClick(e); }} onkeydown={(e) => { if (e.key === 'Escape') closeLinkContextMenu(); }}></div>
				{:else}
					{#if $sourceMode}
						<SourceEditor controller={source} readonly={showHistory || $readOnly || !!$viewerNote || externalOperationBusy} numbers={$appConfig?.show_line_numbers ?? false} />
					{:else}
						<!-- svelte-ignore a11y_no_static_element_interactions -->
						<div class="tiptap-wrapper" class:large-doc={isLargeDoc} spellcheck="false" bind:this={editorElement} onclick={(e) => { closeLinkContextMenu(); handleEditorClick(e); }} onkeydown={(e) => { if (e.key === 'Escape') closeLinkContextMenu(); }} oncontextmenu={handleEditorContextMenu}></div>
					{/if}
				{/if}
			</div>

			{#if showHistory}
				<HistoryReview raw={historyPreview} current={historyWorking}
					selected={historySelected ? formatVersionDate(historySelected.timestamp) : null}
					loading={historyLoading || historyPreviewLoading} error={historyError ?? historyPreviewError}
					canRestore={historyPreview !== null && !historyPreviewLoading && canInsert(captureOrigin())}
					onRetry={() => { if (historySelected) void previewVersion(historySelected); else void loadHistory(); }}
					onRestore={restoreVersion} renderSnapshot={(body) => markdownToHtml(body, true)} />
				<div class="history-panel">
					<div class="history-header">
						<h3>Version History</h3>
						<div class="history-header-actions">
							<button class="history-create-btn" onclick={handleCreateVersion} title="Save current state as a version">
								<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
									<line x1="12" y1="5" x2="12" y2="19" />
									<line x1="5" y1="12" x2="19" y2="12" />
								</svg>
							</button>
							<button class="history-close" onclick={closeHistory} aria-label="Close version history">
								<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
									<line x1="18" y1="6" x2="6" y2="18" />
									<line x1="6" y1="6" x2="18" y2="18" />
								</svg>
							</button>
						</div>
					</div>
					{#if historyLoading}
						<div class="history-empty" role="status">Loading...</div>
					{:else if historyError}
						<div class="history-empty" role="alert">
							<p>{historyError}</p>
							<button class="history-retry-btn" onclick={loadHistory}>Retry</button>
						</div>
					{:else if historyVersions.length === 0}
						<div class="history-empty">No versions yet. Versions are created automatically as you edit (at least 5 minutes apart).</div>
					{:else}
						<div class="history-picker">
							<label for="history-version-picker">Version</label>
							<select id="history-version-picker" aria-label="Select snapshot version"
								value={historySelected?.timestamp ?? ''}
								onchange={(event) => { const version = historyVersions.find((v) => v.timestamp === event.currentTarget.value); if (version) void previewVersion(version); }}>
								{#each historyVersions as version (version.timestamp)}
									<option value={version.timestamp}>{formatVersionDate(version.timestamp)}</option>
								{/each}
							</select>
						</div>
						<div class="history-list">
							{#each historyVersions as v (v.timestamp)}
								<button
									class="history-item"
									class:active={historySelected?.timestamp === v.timestamp}
									aria-pressed={historySelected?.timestamp === v.timestamp}
									onclick={() => previewVersion(v)}
									title={v.timestamp}
								>
									<span class="history-date">{formatVersionDate(v.timestamp)}</span>
									<span class="history-size">{formatVersionSize(v.size)}</span>
								</button>
							{/each}
						</div>
					{/if}
				</div>
			{/if}
			{#if showOutline && !showHistory}
				<ResizeHandle onResize={handleOutlineResize} label="Resize outline" value={$outlineWidth} min={160} max={500} controls={outlinePanelId} />
				<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
				<nav id={outlinePanelId} class="outline-panel" style="width: {$outlineWidth}px" aria-label="Note outline"
					onkeydown={(event) => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); void closeOutline(); } }}>
					<div class="outline-header">
						<h3>Outline</h3>
						<button class="outline-close" onclick={closeOutline} aria-label="Close outline">
							<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
								<line x1="18" y1="6" x2="6" y2="18" />
								<line x1="6" y1="6" x2="18" y2="18" />
							</svg>
						</button>
					</div>
					{#if outlineHeadings.length === 0}
						<div class="outline-empty">No headings in this note.</div>
					{:else}
						<ul class="outline-list" bind:this={outlineList}>
							{#each outlineHeadings as h, i}
								<li>
									<button
										class="outline-item outline-level-{h.level}"
										class:current={i === outlineActive}
										aria-current={i === outlineActive ? 'location' : undefined}
										aria-label={`Heading level ${h.level}: ${h.text.trim() || 'Untitled heading'}`}
										title={h.text.trim() || 'Untitled heading'}
										onclick={() => scrollToHeading(h)}
									>
										{h.text.trim() || 'Untitled heading'}
									</button>
								</li>
							{/each}
						</ul>
					{/if}
				</nav>
			{/if}
			{#if showInfo && $activeNote && !showHistory}
				<div class="info-panel">
					<div class="info-panel-header">
						<span class="info-panel-title">Note Info</span>
						<button class="info-close-btn" onclick={() => showInfo = false} aria-label="Close note info">
							<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
								<line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
							</svg>
						</button>
					</div>

					<div class="info-section">
						<div class="info-section-label">Statistics</div>
						<div class="info-row">
							<span class="info-key">Words</span>
							<span class="info-value">{wordCount.toLocaleString()}</span>
						</div>
						<div class="info-row">
							<span class="info-key">Characters</span>
							<span class="info-value">{charCount.toLocaleString()}</span>
						</div>
						<div class="info-row">
							<span class="info-key">Reading time</span>
							<span class="info-value">{wordCount < 200 ? '< 1 min' : `${Math.ceil(wordCount / 200)} min`}</span>
						</div>
					</div>

					<div class="info-section info-details">
						<div class="info-section-label">Details</div>
						<div class="info-row">
							<span class="info-key">Modified</span>
							<span class="info-value" title={$activeNote.meta.modified}>{formatVersionDate($activeNote.meta.modified)}</span>
						</div>
						<div class="info-row">
							<span class="info-key">Created</span>
							<span class="info-value" title={$activeNote.meta.created}>{formatVersionDate($activeNote.meta.created)}</span>
						</div>
						{#if noteFolder}
						<div class="info-row">
							<span class="info-key">Location</span>
							<span class="info-value info-value-path" title={noteFolder}>{noteFolder}</span>
						</div>
						{/if}
						{#if $activeNote.meta.tags?.length > 0}
						<div class="info-row info-row-tags">
							<span class="info-key">Tags</span>
							<span class="info-value info-tags">
								{#each $activeNote.meta.tags as tag, i (tagIterationKey(tag, i))}
									<TagLabel name={tag} size={11} tone="muted" />
								{/each}
							</span>
						</div>
						{/if}
						{#if !isMobile && $activeNotePath}
						<div class="info-note-path">
							<div class="info-note-path-header">
								<span class="info-key">Path on disk</span>
								<button
									type="button"
									class="info-copy-path-btn"
									class:copied={infoPathCopyState === 'copied'}
									class:error={infoPathCopyState === 'error'}
									onclick={copyActiveNotePath}
									aria-live="polite"
									title="Copy note path"
								>
									{#if infoPathCopyState === 'copied'}
										<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="m5 12 4 4L19 6" /></svg>
										Copied
									{:else if infoPathCopyState === 'error'}
										Copy failed
									{:else}
										<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="11" height="11" rx="2" /><path d="M15 9V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h3" /></svg>
										Copy
									{/if}
								</button>
							</div>
							<code class="info-note-path-value" title={$activeNotePath}>{$activeNotePath}</code>
						</div>
						{/if}
					</div>

					{#key $activeNotePath}
						<details class="info-section info-frontmatter">
							<summary class="info-section-label">Frontmatter</summary>
							<p class="info-frontmatter-caption">Last loaded or saved YAML. Unsaved changes are not shown.</p>
							{#if infoFrontmatter}
								<pre aria-label="Saved frontmatter">{infoFrontmatter}</pre>
							{:else}
								<p class="info-frontmatter-caption">No frontmatter in this note.</p>
							{/if}
						</details>
					{/key}

					{#if onMoveToTrash && $viewMode !== 'trash'}
						<div class="info-section info-actions">
							<button type="button" class="info-trash-btn" onclick={moveOpenNoteToTrash} disabled={trashingNote}>
								<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
									<path d="M3 6h18"/><path d="M19 6v14H5V6"/><path d="M8 6V4h8v2"/><path d="M10 11v5"/><path d="M14 11v5"/>
								</svg>
								{trashingNote ? 'Moving to Trash...' : 'Move to Trash'}
							</button>
						</div>
					{/if}

				</div>
			{/if}
			</div>
		</div>

		{#if editorReady && !$sourceMode && !$viewerNote && !showHistory}
			<!-- svelte-ignore a11y_no_static_element_interactions -->
			<div
				class="editor-formatting-bar"
				class:expanded={moreOpen}
				class:customizing={toolbarDraft !== null}
				bind:this={formattingBar}
				style={isMobile && keyboardHeight > 0 ? `bottom: ${keyboardHeight}px;` : ''}
				onclick={handleFormattingBarClick}
				onclickcapture={captureToolbarClick}
				onkeydown={handleFormattingEscape}
				onkeydowncapture={customizeToolbarKeydown}
				onfocusin={selectToolbarControl}
				onpointerdowncapture={startToolbarDrag}
				onpointerdown={preserveFormattingFocus}
				onmousedown={preserveFormattingFocus}
			>
				{#if toolbarDraft}<span class="fmt-area-label">Toolbar</span>{/if}
				<div class="fmt-main-controls" class:drop-target={toolbarDrop?.area === 'main'} data-toolbar-area="main">
					{#if toolbarDraft}
						{#each toolbarDraft.main as id (id)}
							{@render editableToolbarControl(id)}
						{:else}
							<span class="fmt-empty-drop">Drop buttons here</span>
						{/each}
					{:else}
						{#each formattingGroups as group, index}
							{#if index > 0}<div class="fmt-sep"></div>{/if}
							{#each group as id (id)}
								{@render toolbarControl(id)}
							{/each}
						{/each}
					{/if}
				</div>
				{#if !isMobile}
					<button class="fmt-btn fmt-more-button" class:active={moreOpen} bind:this={moreButton} title="More formatting" aria-label="More formatting" aria-expanded={moreOpen} aria-controls={morePanelId} onclick={toggleMore} onkeydown={moreKeydown}>
						<svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><circle cx="5" cy="12" r="1.8"/><circle cx="12" cy="12" r="1.8"/><circle cx="19" cy="12" r="1.8"/></svg>
					</button>
					{#if moreOpen}
						<div class="fmt-more-panel" class:customizing={toolbarDraft !== null} id={morePanelId} bind:this={morePanel} role="region" aria-label={toolbarDraft ? 'Customize toolbar' : 'More formatting'}>
							{#if toolbarDraft}
								<div class="fmt-customize-heading">
									<strong>Customize toolbar</strong>
									<p>Drag buttons to reorder or move them into More.</p>
								</div>
								<div class="fmt-area-label">More</div>
								<div class="fmt-more-controls" class:drop-target={toolbarDrop?.area === 'more'} data-toolbar-area="more">
									{#each toolbarDraft.more as id (id)}
										{@render editableToolbarControl(id)}
									{:else}
										<span class="fmt-empty-drop">Drop buttons here to keep them in More</span>
									{/each}
								</div>
								<div class="fmt-customize-selection">
									<div class="fmt-selected-label" aria-live="polite">
										<strong>{toolbarSelected ? toolbarLabels[toolbarSelected] : 'Select a button'}</strong>
										{#if toolbarSelectedArea}
											<span>{toolbarSelectedArea === 'main' ? 'Toolbar' : 'More'} · {toolbarSelectedIndex + 1} of {toolbarDraft[toolbarSelectedArea].length}</span>
										{/if}
									</div>
									<div class="fmt-customize-moves">
										<button type="button" disabled={toolbarSaving || !toolbarSelectedArea || toolbarSelectedIndex === 0} onclick={() => toolbarSelectedArea && moveSelectedToolbarButton(toolbarSelectedArea, toolbarSelectedIndex - 1)} aria-label="Move selected button earlier" title="Move earlier">
											<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="m14 6-6 6 6 6"/></svg>
										</button>
										<button type="button" disabled={toolbarSaving || !toolbarSelectedArea || toolbarSelectedIndex === toolbarDraft[toolbarSelectedArea].length - 1} onclick={() => toolbarSelectedArea && moveSelectedToolbarButton(toolbarSelectedArea, toolbarSelectedIndex + 2)} aria-label="Move selected button later" title="Move later">
											<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="m10 6 6 6-6 6"/></svg>
										</button>
										<button type="button" class="fmt-move-area" disabled={toolbarSaving || !toolbarSelectedArea} onclick={() => toolbarDraft && moveSelectedToolbarButton(toolbarSelectedArea === 'main' ? 'more' : 'main', toolbarDraft[toolbarSelectedArea === 'main' ? 'more' : 'main'].length)}>
											Move to {toolbarSelectedArea === 'main' ? 'More' : 'toolbar'}
										</button>
									</div>
									<p class="fmt-customize-hint" id={toolbarHintId}>On a button: ← / → reorder, ↑ to More, ↓ to toolbar.</p>
								</div>
								{#if toolbarConflict}
									<p class="fmt-customize-error" role="alert">The toolbar changed in another window. Cancel and customize again to use the latest layout.</p>
								{:else if toolbarSaveError}
									<p class="fmt-customize-error" role="alert">{toolbarSaveError}</p>
								{/if}
								<div class="fmt-customize-actions">
									<button type="button" class="fmt-customize-reset" disabled={toolbarSaving} onclick={resetToolbarDraft} title="Restore defaults in this draft">Reset</button>
									<button type="button" disabled={toolbarSaving} onclick={() => endToolbarCustomization(!!formattingBar?.contains(document.activeElement))}>Cancel</button>
									<button type="button" class="fmt-customize-done" disabled={toolbarSaving || toolbarConflict || !$appConfig} onclick={saveToolbarDraft}>{toolbarSaving ? 'Saving…' : 'Done'}</button>
								</div>
							{:else}
								{#if toolbarLayout.more.length > 0}
									<div class="fmt-more-controls">
										{#each toolbarLayout.more as id (id)}
											{@render toolbarControl(id)}
										{/each}
									</div>
								{/if}
								<button type="button" class="fmt-customize-entry" disabled={toolbarSaving} onclick={beginToolbarCustomization}>Customize toolbar…</button>
							{/if}
						</div>
					{/if}
					{#if toolbarDrop}
						<div class="fmt-drop-indicator" aria-hidden="true" style={`left:${toolbarDrop.left}px;top:${toolbarDrop.top}px;height:${toolbarDrop.height}px;`}></div>
					{/if}
				{/if}
			</div>
		{/if}
	{/if}

	<!-- Hidden file inputs for Insert dropdown -->
	<input type="file" id="insert-image-input" accept="image/*" style="display:none" onchange={handleAssetInput} />
	<input type="file" id="insert-grid-input" accept="image/*" multiple style="display:none" onchange={handleGridInput} />
	{#if isAndroid}
		<input type="file" id="insert-camera-input" accept="image/*" capture="environment" style="display:none" onchange={handleAssetInput} />
	{/if}
	<input type="file" id="insert-file-input" style="display:none" onchange={handleAssetInput} />
</div>

{#if linkContextMenu}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="link-context-overlay" onclick={(e) => closeFromOverlay(e, closeLinkContextMenu)} onkeydown={(e) => closeOnEscape(e, closeLinkContextMenu)}>
		<div class="link-context-menu" style="left: {linkContextMenu.x}px; top: {linkContextMenu.y}px">
			<div class="link-context-url">{linkContextMenu.href}</div>
			<button onclick={linkMenuOpen}>
				<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<path d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6" />
					<polyline points="15 3 21 3 21 9" />
					<line x1="10" y1="14" x2="21" y2="3" />
				</svg>
				Open Link
			</button>
			{#if isFileLink(linkContextMenu.href)}
			<button onclick={linkMenuSaveAs}>
				<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/>
				</svg>
				Save As...
			</button>
			{/if}
			<button onclick={linkMenuCopy}>
				<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
					<path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1" />
				</svg>
				Copy Link
			</button>
			<button onclick={linkMenuEdit}>
				<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<path d="M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7" />
					<path d="M18.5 2.5a2.121 2.121 0 013 3L12 15l-4 1 1-4 9.5-9.5z" />
				</svg>
				Edit Link
			</button>
			<div class="link-context-sep"></div>
			<button class="danger" onclick={linkMenuRemove}>
				<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<path d="M18 6L6 18M6 6l12 12" />
				</svg>
				Remove Link
			</button>
		</div>
	</div>
{/if}

{#if textContextMenu}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="text-ctx-overlay" onclick={(e) => closeFromOverlay(e, closeTextContextMenu)} onkeydown={(e) => closeOnEscape(e, closeTextContextMenu)}>
		<div class="text-ctx-menu" style="left: {textContextMenu.x}px; top: {textContextMenu.y}px">
			<button onclick={ctxCut}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="6" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><line x1="20" y1="4" x2="8.12" y2="15.88"/><line x1="14.47" y1="14.48" x2="20" y2="20"/><line x1="8.12" y1="8.12" x2="12" y2="12"/></svg>
				Cut
				<span class="text-ctx-shortcut">{modKey}+X</span>
			</button>
			<button onclick={ctxCopy}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1"/></svg>
				Copy
				<span class="text-ctx-shortcut">{modKey}+C</span>
			</button>
			<button onclick={ctxPaste}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M16 4h2a2 2 0 012 2v14a2 2 0 01-2 2H6a2 2 0 01-2-2V6a2 2 0 012-2h2"/><rect x="8" y="2" width="8" height="4" rx="1" ry="1"/></svg>
				Paste
				<span class="text-ctx-shortcut">{modKey}+V</span>
			</button>
			<div class="text-ctx-sep"></div>
			<button onclick={ctxSelectAll}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2"/><path d="M8 7h8M8 12h8M8 17h8"/></svg>
				Select All
				<span class="text-ctx-shortcut">{modKey}+A</span>
			</button>
			<div class="text-ctx-sep"></div>
			<!-- Heading submenu -->
			<div class="text-ctx-submenu-wrap" onmouseenter={() => ctxHeadingSubmenu = true} onmouseleave={() => ctxHeadingSubmenu = false}>
				<button class="has-submenu">
					<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M4 12h16M4 6v12M20 6v12"/></svg>
					Heading
					<svg class="submenu-arrow" width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="9 6 15 12 9 18"/></svg>
				</button>
				{#if ctxHeadingSubmenu}
					<div class="text-ctx-submenu" class:flip-left={textContextMenu?.submenuLeft}>
						<button class:active={isEditorActive('heading', { level: 1 })} onclick={() => ctxSetHeading(1)}>Heading 1</button>
						<button class:active={isEditorActive('heading', { level: 2 })} onclick={() => ctxSetHeading(2)}>Heading 2</button>
						<button class:active={isEditorActive('heading', { level: 3 })} onclick={() => ctxSetHeading(3)}>Heading 3</button>
						<button class:active={isEditorActive('heading', { level: 4 })} onclick={() => ctxSetHeading(4)}>Heading 4</button>
						<div class="text-ctx-sep"></div>
						<button class:active={isEditorActive('paragraph')} onclick={ctxSetParagraph}>Paragraph</button>
					</div>
				{/if}
			</div>
			<div class="text-ctx-sep"></div>
			<button onclick={ctxBold}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1"><path d="M6 4h8a4 4 0 014 4 4 4 0 01-4 4H6zm0 8h9a4 4 0 014 4 4 4 0 01-4 4H6z"/></svg>
				Bold
				<span class="text-ctx-shortcut">{modKey}+B</span>
			</button>
			<button onclick={ctxItalic}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="19" y1="4" x2="10" y2="4"/><line x1="14" y1="20" x2="5" y2="20"/><line x1="15" y1="4" x2="9" y2="20"/></svg>
				Italic
				<span class="text-ctx-shortcut">{modKey}+I</span>
			</button>
			<button onclick={ctxUnderline}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 3v7a6 6 0 006 6 6 6 0 006-6V3"/><line x1="4" y1="21" x2="20" y2="21"/></svg>
				Underline
				<span class="text-ctx-shortcut">{modKey}+U</span>
			</button>
			<button onclick={ctxStrike}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M16 4H9a3 3 0 00-3 3 3 3 0 003 3h6"/><line x1="4" y1="12" x2="20" y2="12"/><path d="M8 20h7a3 3 0 003-3 3 3 0 00-3-3H8"/></svg>
				Strikethrough
			</button>
			<button onclick={ctxHighlight}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9"/><path d="M16.5 3.5a2.12 2.12 0 013 3L7 19l-4 1 1-4L16.5 3.5z"/></svg>
				Highlight
			</button>
			<button onclick={ctxClearFormatting}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m7 21-4.3-4.3c-1-1-1-2.5 0-3.4l9.6-9.6c1-1 2.5-1 3.4 0l5.6 5.6c1 1 1 2.5 0 3.4L13 21"/><path d="M22 21H7"/><path d="m5 11 9 9"/></svg>
				Clear Formatting
			</button>
			<div class="text-ctx-sep"></div>
			<button onclick={ctxLink}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10 13a5 5 0 007.54.54l3-3a5 5 0 00-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 00-7.54-.54l-3 3a5 5 0 007.07 7.07l1.71-1.71"/></svg>
				Add Link
				<span class="text-ctx-shortcut">{modKey}+K</span>
			</button>
			<button onclick={ctxCode}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/></svg>
				Inline Code
			</button>
			<button onclick={ctxCodeBlock}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2"/><polyline points="9 8 5 12 9 16"/><polyline points="15 8 19 12 15 16"/></svg>
				Code Block
			</button>
			<button onclick={ctxBlockquote}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="currentColor"><path d="M3 6h4v4l-2 6H3l2-6H3V6zm10 0h4v4l-2 6h-2l2-6h-2V6z"/></svg>
				Quote
			</button>
			<button onclick={ctxDetails}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2"/><polyline points="10 8 14 12 10 16"/></svg>
				Collapsible Section
			</button>
			<button onclick={ctxCallout}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="5" width="18" height="14" rx="2"/><line x1="7" y1="5" x2="7" y2="19"/></svg>
				Callout
			</button>
			<button onclick={ctxTimestamp}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 7.5V6a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2h6"/><line x1="3" y1="10" x2="21" y2="10"/><line x1="8" y1="2" x2="8" y2="6"/><line x1="16" y1="2" x2="16" y2="6"/><circle cx="18" cy="17" r="4"/><path d="M18 15.5v1.5l1 1"/></svg>
				Date &amp; Time
			</button>
			<div class="text-ctx-sep"></div>
			<button onclick={ctxBulletList}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="8" y1="6" x2="21" y2="6"/><line x1="8" y1="12" x2="21" y2="12"/><line x1="8" y1="18" x2="21" y2="18"/><circle cx="3.5" cy="6" r="1.5" fill="currentColor"/><circle cx="3.5" cy="12" r="1.5" fill="currentColor"/><circle cx="3.5" cy="18" r="1.5" fill="currentColor"/></svg>
				Bullet List
			</button>
			<button onclick={ctxOrderedList}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="10" y1="6" x2="21" y2="6"/><line x1="10" y1="12" x2="21" y2="12"/><line x1="10" y1="18" x2="21" y2="18"/><text x="1" y="8" font-size="8" fill="currentColor" stroke="none" font-weight="600">1</text><text x="1" y="14" font-size="8" fill="currentColor" stroke="none" font-weight="600">2</text><text x="1" y="20" font-size="8" fill="currentColor" stroke="none" font-weight="600">3</text></svg>
				Numbered List
			</button>
			<button onclick={ctxTaskList}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="7" height="7" rx="1.5"/><polyline points="4.5 6.5 6 8 8.5 4.5"/><line x1="13" y1="6.5" x2="21" y2="6.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><line x1="13" y1="17.5" x2="21" y2="17.5"/></svg>
				Task List
			</button>
			{#if $appConfig?.ai_provider}
				<div class="text-ctx-sep"></div>
				<button onclick={openAiMenu}>
					<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						<path d="M12 2a4 4 0 014 4v1a1 1 0 001 1h1a4 4 0 010 8h-1a1 1 0 00-1 1v1a4 4 0 01-8 0v-1a1 1 0 00-1-1H6a4 4 0 010-8h1a1 1 0 001-1V6a4 4 0 014-4z" />
						<circle cx="12" cy="12" r="2" />
					</svg>
					AI Actions
				</button>
			{/if}
		</div>
	</div>
{/if}

{#if tableContextMenu}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="table-ctx-overlay" onclick={(e) => closeFromOverlay(e, closeTableContextMenu)} onkeydown={(e) => closeOnEscape(e, closeTableContextMenu)}>
		<div class="table-ctx-menu" style="left: {tableContextMenu.x}px; top: {tableContextMenu.y}px; max-height: calc(100vh - {tableContextMenu.y}px - 8px); overflow-y: auto;">
			<button onclick={tblAddRowBefore}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18M3 12h18M3 18h18"/><path d="M12 3v3"/><polyline points="9 4.5 12 2 15 4.5"/></svg>
				Add Row Above
			</button>
			<button onclick={tblAddRowAfter}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18M3 12h18M3 18h18"/><path d="M12 21v-3"/><polyline points="9 19.5 12 22 15 19.5"/></svg>
				Add Row Below
			</button>
			<button class="danger" onclick={tblDeleteRow}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 12h18"/><line x1="18" y1="6" x2="6" y2="18"/></svg>
				Delete Row
			</button>
			<div class="table-ctx-sep"></div>
			<button onclick={tblAddColBefore}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 3v18M12 3v18M18 3v18"/><path d="M3 12h3"/><polyline points="4.5 9 2 12 4.5 15"/></svg>
				Add Column Left
			</button>
			<button onclick={tblAddColAfter}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 3v18M12 3v18M18 3v18"/><path d="M21 12h-3"/><polyline points="19.5 9 22 12 19.5 15"/></svg>
				Add Column Right
			</button>
			<button class="danger" onclick={tblDeleteCol}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3v18"/><line x1="18" y1="6" x2="6" y2="18"/></svg>
				Delete Column
			</button>
			<div class="table-ctx-sep"></div>
			<button onclick={tblMergeCells}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2"/><path d="M9 3v18"/><path d="M14 9l-4 3 4 3"/></svg>
				Merge Cells
			</button>
			<button onclick={tblSplitCell}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2"/><path d="M12 3v18"/><path d="M8 9l4 3-4 3"/><path d="M16 9l-4 3 4 3"/></svg>
				Split Cell
			</button>
			<div class="table-ctx-sep"></div>
			<button onclick={tblToggleHeaderRow}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2"/><line x1="3" y1="9" x2="21" y2="9"/><rect x="4" y="4" width="16" height="4" rx="1" fill="currentColor" opacity="0.2"/></svg>
				Toggle Header Row
			</button>
			<button onclick={tblToggleHeaderCol}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2"/><line x1="9" y1="3" x2="9" y2="21"/><rect x="4" y="4" width="4" height="16" rx="1" fill="currentColor" opacity="0.2"/></svg>
				Toggle Header Column
			</button>
			<div class="table-ctx-sep"></div>
			<div class="table-ctx-color-label">Cell Color</div>
			<div class="table-ctx-colors">
				{#each cellColors as color}
					<button
						class="table-ctx-color-swatch"
						title={color.name}
						style="background: {color.value || 'var(--bg-primary)'}; {color.value === '' ? 'border: 1px dashed var(--border-color);' : ''}"
						onclick={() => tblSetCellColor(color.value)}
					></button>
				{/each}
			</div>
			<div class="table-ctx-sep"></div>
			{#if tableContextMenu.hasStyling}
			<button onclick={resetTableToMarkdown} title="Strip cell colors, merged cells, and custom widths, output as plain markdown">
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12a9 9 0 11-3-6.7"/><polyline points="21 4 21 10 15 10"/></svg>
				Reset to Markdown
			</button>
			<div class="table-ctx-sep"></div>
			{/if}
			<button class="danger" onclick={tblDeleteTable}>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14a2 2 0 01-2 2H8a2 2 0 01-2-2L5 6"/><path d="M10 11v6M14 11v6"/><path d="M9 6V4a1 1 0 011-1h4a1 1 0 011 1v2"/></svg>
				Delete Table
			</button>
		</div>
	</div>
{/if}

{#if imageToolbar}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="img-toolbar-overlay" onclick={(e) => closeFromOverlay(e, () => (imageToolbar = null))} onkeydown={(e) => closeOnEscape(e, () => (imageToolbar = null))}>
		<div class="img-toolbar" class:mobile-viewer-toolbar={isAndroid} style="left: {imageToolbar.x}px; top: {imageToolbar.y}px">
			<button class:active={imageToolbar.size === 'small'} onclick={() => setImageSize('small')} title="Small (33%)">S</button>
			<button class:active={imageToolbar.size === 'medium'} onclick={() => setImageSize('medium')} title="Medium (50%)">M</button>
			<button class:active={imageToolbar.size === 'full'} onclick={() => setImageSize('full')} title="Full width">L</button>
			<button class:active={!!imageToolbar.title} onclick={openImageTitleModal} title={imageToolbar.title ? `Hover text: ${imageToolbar.title}` : 'Set hover text'} aria-label="Image hover text">T</button>
			<span class="img-toolbar-sep"></span>
			{#if imageToolbar.inGrid}
				<button onclick={() => moveGridImage(-1)} title="Move left in grid" aria-label="Move left in grid">←</button>
				<button onclick={() => moveGridImage(1)} title="Move right in grid" aria-label="Move right in grid">→</button>
			{:else}
				<button onclick={wrapNearbyImagesInGrid} title="Group neighboring images into a grid">Grid</button>
			{/if}
			{#if isAndroid}
				<span class="img-toolbar-sep"></span>
				<button class="img-toolbar-view" onclick={openImageViewer} title="View and zoom image" aria-label="View and zoom image">
					<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M8 3H5a2 2 0 00-2 2v3M16 3h3a2 2 0 012 2v3M8 21H5a2 2 0 01-2-2v-3M16 21h3a2 2 0 002-2v-3"/></svg>
				</button>
			{/if}
			{#if !isMobile && !imageToolbar.src.startsWith('imgproxy:') && !imageToolbar.src.startsWith('http://imgproxy.localhost') && !imageToolbar.src.startsWith('https://imgproxy.localhost')}
				<span class="img-toolbar-sep"></span>
				<button onclick={copyImageToClipboard} title="Copy image">
					<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1"/></svg>
				</button>
				<button onclick={openImageInApp} title="Open in default app">
					<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>
				</button>
			{/if}
		</div>
	</div>
{/if}

{#if imageViewer}
	<ImageViewer src={imageViewer.src} alt={imageViewer.alt} onclose={() => (imageViewer = null)} />
{/if}

{#if imageTitleModal}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="math-modal-overlay" onclick={(e) => closeFromOverlay(e, cancelImageTitleModal)} onkeydown={(e) => closeOnEscape(e, cancelImageTitleModal)}>
		<div class="math-modal" role="dialog" aria-modal="true" aria-label="Image hover text" tabindex="-1">
			<div class="math-modal-header">
				<span>Image hover text</span>
				<button type="button" class="math-modal-close" onclick={cancelImageTitleModal} aria-label="Close">
					<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
				</button>
			</div>
			<input
				class="math-modal-input img-title-input"
				placeholder="Shown when hovering the image. Leave empty to remove it."
				bind:value={imageTitleModal.title}
				onkeydown={(e) => {
					if (e.key === 'Enter') { e.preventDefault(); commitImageTitleModal(); }
					if (e.key === 'Escape') { e.preventDefault(); cancelImageTitleModal(); }
				}}
				use:autofocus
			/>
			<div class="math-modal-footer">
				<span class="math-modal-hint">Saved as ![alt](image "hover text") · Enter to apply · Esc to cancel</span>
				<div class="math-modal-actions">
					<button type="button" onclick={cancelImageTitleModal}>Cancel</button>
					<button type="button" class="primary" onclick={commitImageTitleModal}>Apply</button>
				</div>
			</div>
		</div>
	</div>
{/if}

{#if copyToast}
	<div class="copy-toast" class:done={copyToast === 'done'}>
		{#if copyToast === 'copying'}
			<svg class="copy-toast-spinner" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
				<circle cx="12" cy="12" r="10" opacity="0.25" />
				<path d="M12 2a10 10 0 019.95 9" />
			</svg>
			Copying...
		{:else}
			<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
				<polyline points="20 6 9 17 4 12" />
			</svg>
			Copied
		{/if}
	</div>
{/if}

{#if mathModal}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="math-modal-overlay" onclick={(e) => closeFromOverlay(e, cancelMathModal)} onkeydown={(e) => closeOnEscape(e, cancelMathModal)}>
		<div class="math-modal" role="dialog" aria-modal="true" aria-label="Math editor" tabindex="-1">
			<div class="math-modal-header">
				<span>{mathModal.editPos !== null ? 'Edit' : 'Insert'} {mathModal.kind === 'block' ? 'Math Block' : 'Inline Math'}</span>
				<button type="button" class="math-modal-close" onclick={cancelMathModal} aria-label="Close">
					<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
				</button>
			</div>
			<textarea
				class="math-modal-input"
				placeholder="LaTeX, e.g. E = mc^2"
				bind:value={mathModal.tex}
				onkeydown={(e) => {
					if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') { e.preventDefault(); commitMathModal(); }
					if (e.key === 'Escape') { e.preventDefault(); cancelMathModal(); }
				}}
				use:autofocus
			></textarea>
			<div class="math-modal-preview">
				{#if mathModal.tex.trim()}
					<div>{@html renderMathPreview(mathModal.tex, mathModal.kind === 'block')}</div>
				{:else}
					<span class="math-modal-preview-empty">Preview appears here…</span>
				{/if}
			</div>
			<div class="math-modal-footer">
				<span class="math-modal-hint">{modKey}+Enter to {mathModal.editPos !== null ? 'update' : 'insert'} · Esc to cancel</span>
				<div class="math-modal-actions">
					<button type="button" onclick={cancelMathModal}>Cancel</button>
					<button type="button" class="primary" onclick={commitMathModal} disabled={!mathModal.tex.trim()}>
						{mathModal.editPos !== null ? 'Update' : 'Insert'}
					</button>
				</div>
			</div>
		</div>
	</div>
{/if}

{#if secretModal}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="math-modal-overlay" onclick={(e) => closeFromOverlay(e, cancelSecretModal)} onkeydown={(e) => closeOnEscape(e, cancelSecretModal)}>
		<div class="math-modal secret-modal" role="dialog" aria-modal="true" aria-label="Encrypted secret editor" tabindex="-1">
			<div class="math-modal-header">
				<span>Insert Secret</span>
				<button type="button" class="math-modal-close" onclick={cancelSecretModal} aria-label="Close">
					<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
				</button>
			</div>
			<input
				class="secret-modal-title"
				type="text"
				placeholder="Title"
				bind:value={secretModal.title}
				bind:this={secretTitleInput}
				onkeydown={(e) => {
					if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') { e.preventDefault(); commitSecretModal(); }
					if (e.key === 'Escape') { e.preventDefault(); cancelSecretModal(); }
				}}
			/>
			<textarea
				class="math-modal-input secret-modal-text"
				placeholder="Secret text"
				bind:value={secretModal.text}
				onkeydown={(e) => {
					if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') { e.preventDefault(); commitSecretModal(); }
					if (e.key === 'Escape') { e.preventDefault(); cancelSecretModal(); }
				}}
			></textarea>
			<div class="secret-modal-fields">
				<input
					type="password"
					placeholder="Passphrase"
					autocomplete="new-password"
					bind:value={secretModal.passphrase}
					onkeydown={(e) => {
						if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') { e.preventDefault(); commitSecretModal(); }
						if (e.key === 'Escape') { e.preventDefault(); cancelSecretModal(); }
					}}
				/>
				<input
					type="password"
					placeholder="Confirm passphrase"
					autocomplete="new-password"
					bind:value={secretModal.confirmPassphrase}
					onkeydown={(e) => {
						if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') { e.preventDefault(); commitSecretModal(); }
						if (e.key === 'Escape') { e.preventDefault(); cancelSecretModal(); }
					}}
				/>
			</div>
			{#if secretModal.error}
				<div class="secret-modal-error">{secretModal.error}</div>
			{/if}
			<div class="math-modal-footer">
				<span class="math-modal-hint">Stored as a portable helix-secret markdown block</span>
				<div class="math-modal-actions">
					<button type="button" onclick={cancelSecretModal} disabled={secretModal.busy}>Cancel</button>
					<button type="button" class="primary" onclick={commitSecretModal} disabled={secretModal.busy || !secretModal.text || !secretModal.passphrase || !secretModal.confirmPassphrase}>
						{secretModal.busy ? 'Encrypting...' : 'Encrypt'}
					</button>
				</div>
			</div>
		</div>
	</div>
{/if}

{#if viewerImportPickerOpen}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="viewer-import-overlay" onclick={(e) => closeFromOverlay(e, () => (viewerImportPickerOpen = false))} onkeydown={(e) => closeOnEscape(e, () => (viewerImportPickerOpen = false))}>
		<div class="viewer-import-picker" role="dialog" aria-modal="true" aria-label="Import image to folder" tabindex="-1">
			<div class="viewer-import-header">
				<span>Import to folder</span>
				<button type="button" class="viewer-import-close" onclick={() => (viewerImportPickerOpen = false)} aria-label="Close">
					<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
				</button>
			</div>
			<div class="viewer-import-list">
				<button type="button" class="viewer-import-item" onclick={() => viewerImportTo('')} disabled={viewerImportBusy}>
					<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>
					Vault root
				</button>
				{#each viewerFlatNotebooks as nb (nb.path)}
					<button type="button" class="viewer-import-item" style="padding-left: {12 + nb.depth * 16}px" onclick={() => viewerImportTo(nb.path)} disabled={viewerImportBusy}>
						<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>
						{nb.name}
					</button>
				{/each}
			</div>
		</div>
	</div>
{/if}

{#if tagMenu && $activeNote}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="tag-menu-overlay" onclick={(e) => closeFromOverlay(e, () => (tagMenu = null))} onkeydown={(e) => closeOnEscape(e, () => (tagMenu = null))}>
		<div class="tag-menu" style="left: {tagMenu.x}px; top: {tagMenu.y}px">
			{#if $activeNote.meta.tags.length > 0}
				<div class="tag-menu-list">
					{#each $activeNote.meta.tags as tag, i (tagIterationKey(tag, i))}
						<span class="tag-menu-chip">
							<TagLabel name={tag} size={12} />
							<button class="tag-menu-remove" onclick={() => removeActiveNoteTag(tag)} title="Remove tag" aria-label="Remove tag">
								<svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
							</button>
						</span>
					{/each}
				</div>
			{/if}
			<TagSuggestInput
				existing={$activeNote.meta.tags}
				placeholder="Add tag..."
				onsubmit={(t) => addActiveNoteTag(t)}
				oncancel={() => (tagMenu = null)}
			/>
		</div>
	</div>
{/if}

{#if codeLangDropdown}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="code-lang-overlay" onclick={(e) => closeFromOverlay(e, closeCodeLangDropdown)} onkeydown={(e) => closeOnEscape(e, closeCodeLangDropdown)}>
		<div class="code-lang-dropdown" style="left: {codeLangDropdown.x}px; top: {codeLangDropdown.y}px">
			<input class="code-lang-search" type="text" placeholder="Search..." bind:this={codeLangInput} bind:value={codeLangSearch} onkeydown={(e) => {
				if (e.key === 'Enter' && codeLangFiltered.length > 0) { e.preventDefault(); e.stopPropagation(); selectCodeLang(codeLangFiltered[0]); }
				if (e.key === 'Escape') closeCodeLangDropdown();
			}} />
			{#if !codeLangSearch}
				<button
					class="code-lang-option"
					class:active={codeLangDropdown.current === ''}
					onclick={() => selectCodeLang('')}
				>auto</button>
			{/if}
			{#each codeLangFiltered as lang}
				<button
					class="code-lang-option"
					class:active={codeLangDropdown.current === lang}
					onclick={() => selectCodeLang(lang)}
				>{lang}</button>
			{/each}
			{#if codeLangFiltered.length === 0}
				<div class="code-lang-empty">No match</div>
			{/if}
		</div>
	</div>
{/if}

{#if slashMenu}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="slash-menu-overlay" onclick={(e) => closeFromOverlay(e, closeSlashMenu)} onkeydown={(e) => closeOnEscape(e, closeSlashMenu)}>
		<div class="slash-menu" style="left: {slashMenu.x}px; top: {slashMenu.y}px">
			{#if slashTablePicker}
				<div class="slash-table-picker">
					<div class="slash-table-picker-grid">
						{#each Array(8) as _, r}
							{#each Array(10) as _, c}
								<!-- svelte-ignore a11y_no_static_element_interactions -->
								<div
									class="table-picker-cell"
									class:active={r < slashTableHover.rows && c < slashTableHover.cols}
									onmouseenter={() => slashTableHover = { rows: r + 1, cols: c + 1 }}
									onmousedown={(e) => { e.preventDefault(); slashInsertTable(r + 1, c + 1); }}
								></div>
							{/each}
						{/each}
					</div>
					<div class="slash-table-picker-label">
						{slashTableHover.rows > 0 ? `${slashTableHover.rows} x ${slashTableHover.cols}` : 'Select table size'}
					</div>
				</div>
			{:else if slashColorPicker}
				<div class="slash-color-picker">
					<div class="slash-color-swatches">
						{#each colorPresets as c}
							<button class="slash-color-swatch" style="background: {c}" title={c} aria-label={c} onmousedown={(e) => { e.preventDefault(); insertColor(c); }}></button>
						{/each}
					</div>
					<div class="slash-color-row">
						<input type="color" class="slash-color-native" value={/^#[0-9a-fA-F]{6}$/.test(slashColorHex) ? slashColorHex : '#4b6abf'} oninput={(e) => { slashColorHex = (e.target as HTMLInputElement).value; }} title="Pick a color" />
						<input
							type="text"
							class="slash-color-input"
							bind:this={slashColorInputEl}
							value={slashColorHex}
							placeholder="#hex or rgb(...)"
							oninput={(e) => { slashColorHex = (e.target as HTMLInputElement).value; }}
							onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); insertColor(slashColorHex); } else if (e.key === 'Escape') { e.preventDefault(); closeSlashMenu(); } }}
						/>
						<button class="slash-color-insert" onmousedown={(e) => { e.preventDefault(); insertColor(slashColorHex); }}>Insert</button>
					</div>
				</div>
			{:else if slashFiltered.length === 0}
				<div class="slash-menu-empty">No matching commands</div>
			{:else}
				{#each slashFiltered as cmd, i}
					<button
						class="slash-menu-item"
						class:selected={i === slashSelectedIndex}
						onmouseenter={() => slashSelectedIndex = i}
						onmousedown={(e) => { e.preventDefault(); executeSlashCommand(i); }}
					>
						<span class="slash-menu-icon">{@html cmd.icon}</span>
						<span class="slash-menu-label">{cmd.label}</span>
					</button>
				{/each}
			{/if}
		</div>
	</div>
{/if}

{#if taskMetaMenu}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="slash-menu-overlay" onclick={(e) => closeFromOverlay(e, closeTaskMetaMenu)} onkeydown={(e) => closeOnEscape(e, closeTaskMetaMenu)}>
		<div class="slash-menu" style="left: {taskMetaMenu.x}px; top: {taskMetaMenu.y}px">
			{#if taskMetaFiltered.length === 0}
				<div class="slash-menu-empty">No match</div>
			{:else}
				{#each taskMetaFiltered as item, i}
					<button
						class="slash-menu-item"
						class:selected={i === taskMetaSelectedIndex}
						onmouseenter={() => taskMetaSelectedIndex = i}
						onmousedown={(e) => { e.preventDefault(); selectTaskMeta(i); }}
					>
						<span class="slash-menu-icon">{@html item.icon}</span>
						<span class="slash-menu-label">{item.label}</span>
					</button>
				{/each}
			{/if}
		</div>
	</div>
{/if}

{#if taskDuePicker}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="slash-menu-overlay" onclick={(e) => closeFromOverlay(e, () => { taskDuePicker = null; editor?.commands.focus(); })} onkeydown={(e) => closeOnEscape(e, () => { taskDuePicker = null; editor?.commands.focus(); })}>
		<input
			type="date"
			class="task-due-input"
			bind:this={taskDueInputEl}
			style="left: {taskDuePicker.x}px; top: {taskDuePicker.y}px"
			onchange={(e) => applyTaskDue((e.currentTarget as HTMLInputElement).value)}
			onkeydown={(e) => { if (e.key === 'Escape') { taskDuePicker = null; editor?.commands.focus(); } }}
		/>
	</div>
{/if}

{#if wikiLinkMenu}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="wiki-link-overlay" onclick={(e) => closeFromOverlay(e, closeWikiLinkMenu)} onkeydown={(e) => closeOnEscape(e, closeWikiLinkMenu)}>
		<div class="wiki-link-menu" style="left: {wikiLinkMenu.x}px; top: {wikiLinkMenu.y}px">
			{#if wikiLinkFiltered.length === 0}
				<div class="wiki-link-empty">
					{wikiLinkMenu.query ? 'No matching notes' : 'Type to search notes...'}
				</div>
			{:else}
				{#each wikiLinkFiltered.slice(0, 12) as entry, i}
					<button
						class="wiki-link-item"
						class:selected={i === wikiLinkSelectedIndex}
						onmouseenter={() => wikiLinkSelectedIndex = i}
						onmousedown={(e) => { e.preventDefault(); if (wikiLinkDisambigEntries) { insertWikiLink({ ...entry, title: wikiLinkDisambigDisplay || entry.title }, wikiLinkDisambigRef || undefined); } else { insertWikiLink(entry); } }}
					>
						<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M13 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V9z"/><polyline points="13 2 13 9 20 9"/></svg>
						<span class="wiki-link-title-col">
							<span class="wiki-link-title">{entry.title}</span>
							{#if wikiLinkFolderPath(entry)}
								<span class="wiki-link-folder">{wikiLinkFolderPath(entry)}</span>
							{/if}
						</span>
					</button>
				{/each}
			{/if}
		</div>
	</div>
{/if}

{#if wikiLinkNavDisambig}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="wiki-link-overlay" onclick={(e) => closeFromOverlay(e, () => wikiLinkNavDisambig = null)} onkeydown={(e) => closeOnEscape(e, () => wikiLinkNavDisambig = null)}>
		<div class="wiki-link-menu" style="left: {wikiLinkNavDisambig.x}px; top: {wikiLinkNavDisambig.y}px">
			<div class="wiki-link-disambig-header">Multiple notes found - choose one:</div>
			{#each wikiLinkNavDisambig.entries as entry, i}
				<button
					class="wiki-link-item"
					class:selected={i === wikiLinkNavDisambigIndex}
					onmouseenter={() => wikiLinkNavDisambigIndex = i}
					onmousedown={(e) => { e.preventDefault(); navigateToWikiLinkDirect(entry); }}
				>
					<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M13 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V9z"/><polyline points="13 2 13 9 20 9"/></svg>
					<span class="wiki-link-title-col">
						<span class="wiki-link-title">{entry.title}</span>
						<span class="wiki-link-folder">{wikiLinkFolderPath(entry) || '(vault root)'}</span>
					</span>
				</button>
			{/each}
		</div>
	</div>
{/if}

{#if aiMenu}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="ai-menu-overlay" class:mobile={isMobile} onclick={(e) => closeFromOverlay(e, closeAiMenu)} onkeydown={(e) => closeOnEscape(e, closeAiMenu)}>
		<div class="ai-menu" class:mobile={isMobile} style={isMobile ? '' : `left: ${aiMenu.x}px; top: ${aiMenu.y}px`}>
			{#if aiResult !== null || aiLoading}
				<!-- Result view -->
				<div class="ai-result-header">
					<span class="ai-result-title">
						{#if aiLoading}
							<svg class="ai-spinner" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10" opacity="0.25" /><path d="M12 2a10 10 0 019.95 9" /></svg>
							Generating...
						{:else}
							AI Result
						{/if}
					</span>
					<button class="ai-result-close" onclick={closeAiMenu} aria-label="Close AI menu">
						<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
					</button>
				</div>
				{#if aiError}
					<div class="ai-error">{aiError}</div>
				{:else}
					<div class="ai-result-body">{aiResult}</div>
				{/if}
				{#if !aiLoading && aiResult && !aiError}
					<div class="ai-result-actions">
						<button class="ai-action-btn apply" onclick={aiApplyResult}>
							<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
							{aiEmptyNote ? 'Insert Note' : aiWholeNote ? 'Apply to Note' : 'Replace'}
						</button>
						<button class="ai-action-btn discard" onclick={aiDiscard}>
							<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
							Discard
						</button>
					</div>
				{/if}
			{:else if aiShowCustom}
				<!-- Custom prompt input -->
				<div class="ai-custom-header">
					<button class="ai-back-btn" onclick={() => aiShowCustom = false} aria-label="Back to AI actions">
						<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="15 18 9 12 15 6"/></svg>
					</button>
					<span>Custom Prompt</span>
				</div>
				<div class="ai-custom-body">
					<textarea
						class="ai-custom-input"
						placeholder="Tell AI what to do with the selected text..."
						bind:value={aiCustomPrompt}
						onkeydown={(e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); runAiAction('custom', aiCustomPrompt); } }}
						use:autofocus
					></textarea>
					<button class="ai-custom-submit" onclick={() => runAiAction('custom', aiCustomPrompt)} disabled={!aiCustomPrompt.trim()}>
						<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="22" y1="2" x2="11" y2="13" /><polygon points="22 2 15 22 11 13 2 9 22 2" /></svg>
						Send
					</button>
				</div>
			{:else if aiTranslateMenu}
				<!-- Translate submenu -->
				<div class="ai-custom-header">
					<button class="ai-back-btn" onclick={() => aiTranslateMenu = false} aria-label="Back to AI actions">
						<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="15 18 9 12 15 6"/></svg>
					</button>
					<span>Translate to</span>
				</div>
				<button class="ai-menu-item" onclick={() => runAiAction('translate_en')}>English</button>
				<button class="ai-menu-item" onclick={() => runAiAction('translate_nl')}>Dutch</button>
				<button class="ai-menu-item" onclick={() => runAiAction('translate_de')}>German</button>
				<button class="ai-menu-item" onclick={() => runAiAction('translate_fr')}>French</button>
				<button class="ai-menu-item" onclick={() => runAiAction('translate_es')}>Spanish</button>
			{:else if aiEmptyNote}
				<!-- Empty note - generate from prompt -->
				<div class="ai-menu-label">Generate Note</div>
				<div class="ai-custom-body">
					<textarea
						class="ai-custom-input"
						placeholder="Describe the note you want to create..."
						bind:value={aiCustomPrompt}
						onkeydown={(e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); runAiAction('custom', aiCustomPrompt); } }}
						use:autofocus
					></textarea>
					<button class="ai-custom-submit" onclick={() => runAiAction('custom', aiCustomPrompt)} disabled={!aiCustomPrompt.trim()}>
						<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 8V4l-2-2"/><rect x="4" y="8" width="16" height="12" rx="2"/><path d="M2 14h2"/><path d="M20 14h2"/><path d="M9 13v2"/><path d="M15 13v2"/></svg>
						Generate
					</button>
				</div>
			{:else}
				<!-- Action list -->
				<div class="ai-menu-label">{aiWholeNote ? 'AI Actions (Entire Note)' : 'AI Actions'}</div>
				<button class="ai-menu-item" onclick={() => runAiAction('improve')}>
					<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9"/><path d="M16.5 3.5a2.12 2.12 0 013 3L7 19l-4 1 1-4L16.5 3.5z"/></svg>
					Improve Writing
				</button>
				<button class="ai-menu-item" onclick={() => runAiAction('fix_grammar')}>
					<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
					Fix Grammar
				</button>
				<button class="ai-menu-item" onclick={() => runAiAction('shorter')}>
					<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 12h16"/><path d="M4 6h10"/></svg>
					Make Shorter
				</button>
				<button class="ai-menu-item" onclick={() => runAiAction('longer')}>
					<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 6h16"/><path d="M4 12h16"/><path d="M4 18h10"/></svg>
					Make Longer
				</button>
				<div class="ai-menu-sep"></div>
				<button class="ai-menu-item" onclick={() => runAiAction('professional')}>
					<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="7" width="20" height="14" rx="2" ry="2"/><path d="M16 7V5a2 2 0 00-2-2h-4a2 2 0 00-2 2v2"/></svg>
					Professional Tone
				</button>
				<button class="ai-menu-item" onclick={() => runAiAction('friendly')}>
					<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><path d="M8 14s1.5 2 4 2 4-2 4-2"/><line x1="9" y1="9" x2="9.01" y2="9"/><line x1="15" y1="9" x2="15.01" y2="9"/></svg>
					Friendly Tone
				</button>
				<div class="ai-menu-sep"></div>
				<button class="ai-menu-item" onclick={() => runAiAction('summarize')}>
					<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="4" y1="6" x2="20" y2="6"/><line x1="4" y1="10" x2="16" y2="10"/><line x1="4" y1="14" x2="12" y2="14"/></svg>
					Summarize
				</button>
				<button class="ai-menu-item" onclick={() => runAiAction('explain')}>
					<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><path d="M9.09 9a3 3 0 015.83 1c0 2-3 3-3 3"/><line x1="12" y1="17" x2="12.01" y2="17"/></svg>
					Explain
				</button>
				<button class="ai-menu-item" onclick={() => aiTranslateMenu = true}>
					<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 8l6 6"/><path d="M4 14l6-6 2-3"/><path d="M2 5h12"/><path d="M7 2h1"/><path d="M22 22l-5-10-5 10"/><path d="M14 18h6"/></svg>
					Translate
					<span class="ai-menu-arrow">
						<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="9 18 15 12 9 6"/></svg>
					</span>
				</button>
				<div class="ai-menu-sep"></div>
				<button class="ai-menu-item" onclick={() => aiShowCustom = true}>
					<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="22" y1="2" x2="11" y2="13" /><polygon points="22 2 15 22 11 13 2 9 22 2" /></svg>
					Custom Prompt...
				</button>
			{/if}
		</div>
	</div>
{/if}

{#if linkModal}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="link-modal-overlay" onclick={(e) => closeFromOverlay(e, linkModalCancel)} onkeydown={(e) => closeOnEscape(e, linkModalCancel)}>
		<div class="link-modal" role="dialog" aria-modal="true" aria-label="Insert link" tabindex="-1">
			<div class="link-modal-header">
				<svg width="28" height="28" viewBox="0 0 48 48" fill="none">
					<rect width="48" height="48" rx="12" fill="var(--accent)" />
					<circle cx="16" cy="16" r="3.5" fill="white" opacity="0.9" />
					<circle cx="32" cy="16" r="3.5" fill="white" opacity="0.9" />
					<circle cx="16" cy="32" r="3.5" fill="white" opacity="0.9" />
					<circle cx="32" cy="32" r="3.5" fill="white" opacity="0.9" />
					<line x1="19" y1="18" x2="29" y2="30" stroke="white" stroke-width="2" stroke-linecap="round" opacity="0.7" />
					<line x1="29" y1="18" x2="19" y2="30" stroke="white" stroke-width="2" stroke-linecap="round" opacity="0.7" />
				</svg>
				<span>Insert Link</span>
			</div>
			<input
				type="text"
				class="link-modal-input"
				bind:this={linkModalInput}
				bind:value={linkModalUrl}
				oninput={() => { linkSuggestIndex = 0; }}
				onkeydown={(e) => {
					if (linkSuggestFiltered.length > 0) {
						if (e.key === 'ArrowDown') { e.preventDefault(); linkSuggestIndex = Math.min(linkSuggestIndex + 1, linkSuggestFiltered.length - 1); return; }
						if (e.key === 'ArrowUp') { e.preventDefault(); linkSuggestIndex = Math.max(linkSuggestIndex - 1, 0); return; }
						if (e.key === 'Enter') { e.preventDefault(); linkModalSelectNote(linkSuggestFiltered[linkSuggestIndex]); return; }
					} else {
						if (e.key === 'Enter') { e.preventDefault(); linkModalConfirm(); }
					}
					if (e.key === 'Escape') { e.preventDefault(); linkModalCancel(); }
				}}
				placeholder="URL or note name"
			/>
			{#if linkSuggestFiltered.length > 0}
				<div class="link-suggest-list">
					{#each linkSuggestFiltered as entry, i}
						<button
							class="link-suggest-item"
							class:selected={i === linkSuggestIndex}
							onmouseenter={() => linkSuggestIndex = i}
							onmousedown={(e) => { e.preventDefault(); linkModalSelectNote(entry); }}
						>
							<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M13 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V9z"/><polyline points="13 2 13 9 20 9"/></svg>
							<span class="link-suggest-title">{entry.title}</span>
						</button>
					{/each}
				</div>
			{/if}
			<div class="link-modal-actions">
				<button class="link-modal-btn cancel" onclick={linkModalCancel}>Cancel</button>
				<button class="link-modal-btn confirm" onclick={linkModalConfirm}>
					{linkModalUrl ? 'Apply' : 'Remove Link'}
				</button>
			</div>
		</div>
	</div>
{/if}

{#if showGraph}
	<GraphView onclose={() => showGraph = false} onnavigate={(path, title) => { showGraph = false; navigateToWikiLink(path, title); }} />
{/if}

<style>
	.editor-container {
		display: flex;
		flex-direction: column;
		height: 100%;
		background: var(--bg-editor);
	}

	.empty-editor {
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		height: 100%;
		color: var(--text-tertiary);
		gap: 12px;
	}

	.empty-icon {
		opacity: 0.5;
	}

	.empty-editor p {
		font-size: 14px;
	}

	.shortcuts-hint {
		display: flex;
		gap: 16px;
		font-size: 12px;
		margin-top: 8px;
	}

	.shortcuts-hint kbd {
		background: var(--bg-tertiary);
		border: 1px solid var(--border-color);
		border-radius: 3px;
		padding: 1px 5px;
		font-size: 11px;
		font-family: inherit;
	}

	.editor-toolbar {
		display: flex;
		/* In a narrow pane the buttons move to their own row instead of squeezing the title away. */
		flex-wrap: wrap;
		row-gap: 4px;
		align-items: center;
		justify-content: space-between;
		padding: 12px 20px 4px;
		flex-shrink: 0;
	}

	.nav-history-btns {
		display: flex;
		align-items: center;
		gap: 2px;
		margin-right: auto;
		flex-shrink: 0;
	}

	.nav-history-btn {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 24px;
		height: 24px;
		border: none;
		background: none;
		color: var(--text-tertiary);
		border-radius: 5px;
		cursor: pointer;
		padding: 0;
		transition: color 0.15s, background 0.15s;
	}

	.nav-history-btn:hover {
		color: var(--text-primary);
		background: var(--bg-hover);
	}

	.nav-history-btn:disabled {
		opacity: 0.3;
		cursor: default;
		pointer-events: none;
	}

	.editor-title {
		flex: 1;
		/* The title used to be the only thing that could shrink, down to nothing. */
		min-width: min(12rem, 100%);
	}

	.editor-title input {
		width: 100%;
		border: none;
		background: none;
		color: var(--text-primary);
		font-size: 22px;
		font-weight: 700;
		outline: none;
		padding: 0;
		-webkit-user-select: text !important;
		user-select: text !important;
	}

	.editor-title input::placeholder {
		color: var(--text-tertiary);
	}

	.note-meta-bar {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 5px;
		padding: 0 20px 10px;
		flex-shrink: 0;
	}

	.note-folder {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		font-size: 11px;
		color: var(--text-tertiary);
		letter-spacing: 0.01em;
	}

	.note-folder.unfiled {
		opacity: 0.6;
		font-style: italic;
	}

	.path-sep {
		font-size: 10px;
		opacity: 0.5;
	}

	.meta-divider {
		font-size: 11px;
		color: var(--text-tertiary);
		opacity: 0.4;
		user-select: none;
	}

	.note-tags-trigger {
		display: inline-flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 4px;
		border: none;
		background: none;
		padding: 0;
		margin: 0;
		font: inherit;
		color: inherit;
		cursor: pointer;
	}

	.note-tag-link {
		display: inline-flex;
		border: none;
		background: none;
		padding: 0;
		margin: 0;
		font: inherit;
		color: inherit;
		cursor: pointer;
		border-radius: 4px;
	}

	.note-tag-link:hover {
		color: var(--accent);
	}

	.note-tags-add {
		font-size: 11px;
		color: var(--text-tertiary);
		opacity: 0.65;
	}

	.note-tags-trigger:hover .note-tags-add {
		opacity: 1;
		color: var(--accent);
	}

	.toolbar-actions {
		display: flex;
		align-items: center;
		gap: 8px;
		margin-left: auto;
	}

	.save-indicator {
		font-size: 11px;
		color: var(--text-tertiary);
		background: var(--bg-tertiary);
		padding: 2px 8px;
		border-radius: 4px;
	}

	.save-error {
		display: flex;
		align-items: center;
		gap: 12px;
		flex-shrink: 0;
		padding: 8px 16px;
		border-bottom: 1px solid color-mix(in srgb, var(--danger, #e53e3e) 30%, var(--border-color));
		background: color-mix(in srgb, var(--danger, #e53e3e) 8%, var(--bg-primary));
		color: var(--danger, #e53e3e);
		font-size: 12px;
		line-height: 1.5;
	}

	.save-error > span {
		flex: 1;
		min-width: 0;
		overflow-wrap: anywhere;
	}

	.save-error button {
		flex-shrink: 0;
		min-height: 32px;
		padding: 4px 10px;
		border: 1px solid currentColor;
		border-radius: 4px;
		background: transparent;
		color: inherit;
		font: inherit;
		cursor: pointer;
	}

	.save-error button:hover:not(:disabled) {
		background: color-mix(in srgb, var(--danger, #e53e3e) 12%, transparent);
	}

	.save-error button:focus-visible {
		outline: 2px solid currentColor;
		outline-offset: 2px;
	}

	.save-error button:disabled {
		opacity: 0.6;
		cursor: wait;
	}

	.editor-container.mobile .save-error button {
		min-height: 44px;
	}

	.editor-title.search-target-meta,
	.note-tags-trigger.search-target-meta {
		background: var(--accent-light);
		outline: 1px solid var(--accent);
		border-radius: 4px;
	}

	.search-target-notice {
		padding: 6px 12px;
		font-size: 12px;
		color: var(--text-secondary);
		background: var(--accent-light);
		overflow-wrap: anywhere;
	}

	.readonly-indicator {
		font-size: 11px;
		color: var(--accent);
		background: var(--accent-light);
		padding: 2px 8px;
		border-radius: 4px;
		font-weight: 500;
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

	.icon-btn.active {
		color: var(--text-accent);
		background: var(--accent-light);
	}

	.sr-only {
		position: absolute;
		width: 1px;
		height: 1px;
		padding: 0;
		margin: -1px;
		overflow: hidden;
		clip: rect(0, 0, 0, 0);
		border: 0;
	}

	.editor-formatting-bar {
		display: flex;
		align-items: center;
		gap: 2px;
		padding: 6px 20px;
		border-top: 1px solid var(--border-light);
		flex-shrink: 0;
		flex-wrap: wrap;
	}

	.editor-formatting-bar.expanded {
		flex-shrink: 1;
		min-height: 44px;
		max-height: 70%;
		overflow-y: auto;
		overscroll-behavior: contain;
		align-content: flex-start;
	}

	.fmt-main-controls {
		display: contents;
	}

	.fmt-more-button {
		flex-shrink: 0;
	}

	/* More expands above the main row, inside the bar. Only formatting pickers
	   float over the editor; short windows scroll the expanded toolbar in place. */
	.fmt-more-panel {
		order: -1;
		flex: 0 0 100%;
		min-width: 0;
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 8px;
		box-sizing: border-box;
		max-height: min(360px, 45vh);
		overflow: auto;
		overscroll-behavior: contain;
		border-bottom: 1px solid var(--border-light);
		padding: 2px 0 8px;
		animation: fmt-expand 140ms ease-out;
	}

	.fmt-more-controls {
		display: flex;
		flex-wrap: wrap;
		gap: 2px;
		align-content: start;
		justify-content: start;
	}

	.fmt-customize-entry {
		min-height: 30px;
		margin-left: auto;
		padding: 6px 8px;
		border: 0;
		border-radius: 4px;
		background: none;
		color: var(--text-secondary);
		text-align: left;
		font-size: 12px;
		white-space: nowrap;
		cursor: pointer;
	}


	.fmt-customize-entry:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.editor-formatting-bar.customizing {
		background: var(--bg-secondary);
		gap: 8px;
	}

	.customizing .fmt-main-controls,
	.fmt-more-panel.customizing .fmt-more-controls {
		border: 1px dashed var(--border-color);
		border-radius: 6px;
		padding: 5px;
		min-height: 42px;
		box-sizing: border-box;
		overflow: auto;
		overscroll-behavior: contain;
	}

	.customizing .fmt-main-controls {
		display: flex;
		flex: 1;
		min-width: 0;
		flex-wrap: wrap;
		align-items: center;
		gap: 2px;
		max-height: 116px;
	}

	.fmt-more-panel.customizing {
		align-items: stretch;
		flex-direction: column;
		flex-wrap: nowrap;
		padding: 4px 0 10px;
	}

	.fmt-more-panel.customizing .fmt-more-controls {
		flex-shrink: 0;
		max-height: 116px;
	}

	.fmt-area-label {
		font-size: 11px;
		font-weight: 600;
		color: var(--text-tertiary);
		flex-shrink: 0;
	}

	.fmt-empty-drop {
		font-size: 11px;
		color: var(--text-tertiary);
		padding: 7px 4px;
		grid-column: 1 / -1;
		line-height: 1.5;
		pointer-events: none;
	}

	.fmt-control {
		flex-shrink: 0;
		border-radius: 4px;
		touch-action: none;
	}

	.fmt-control .fmt-btn {
		cursor: grab;
		background: none;
		color: var(--text-secondary);
	}

	.fmt-control.selected {
		outline: 2px solid var(--accent);
		outline-offset: -1px;
		background: var(--accent-light);
	}

	.fmt-control.dragging {
		opacity: 0.3;
	}

	.customizing .drop-target {
		border-color: var(--accent);
		background: var(--accent-light);
	}

	.fmt-drop-indicator {
		position: fixed;
		width: 3px;
		border-radius: 2px;
		background: var(--accent);
		z-index: 1000;
		pointer-events: none;
	}

	.fmt-customize-heading {
		display: flex;
		align-items: baseline;
		flex-wrap: wrap;
		gap: 4px 12px;
	}

	.fmt-customize-heading strong {
		font-size: 13px;
		font-weight: 600;
		color: var(--text-primary);
	}

	.fmt-customize-heading p,
	.fmt-customize-hint {
		margin: 5px 0 0;
		font-size: 11px;
		line-height: 1.5;
		color: var(--text-tertiary);
	}

	.fmt-customize-selection {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 8px 16px;
		border-top: 1px solid var(--border-light);
		padding-top: 9px;
	}

	.fmt-selected-label {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		gap: 8px;
		margin-right: auto;
		font-size: 11px;
	}

	.fmt-customize-selection .fmt-customize-hint {
		flex-basis: 100%;
		margin: 0;
	}

	@keyframes fmt-expand {
		from { opacity: 0; }
		to { opacity: 1; }
	}

	@media (prefers-reduced-motion: reduce) {
		.fmt-more-panel { animation: none; }
	}

	.fmt-selected-label strong {
		color: var(--text-primary);
		font-weight: 500;
	}

	.fmt-selected-label span {
		color: var(--text-tertiary);
		white-space: nowrap;
	}

	.fmt-customize-moves,
	.fmt-customize-actions {
		display: flex;
		gap: 5px;
	}

	.fmt-customize-moves button,
	.fmt-customize-actions button {
		display: flex;
		align-items: center;
		justify-content: center;
		min-width: 28px;
		min-height: 28px;
		padding: 4px 8px;
		border: 1px solid var(--border-color);
		border-radius: 5px;
		background: var(--bg-primary);
		color: var(--text-secondary);
		font-size: 11px;
		cursor: pointer;
	}

	.fmt-customize-moves button:hover:not(:disabled),
	.fmt-customize-actions button:hover:not(:disabled) {
		background: var(--bg-hover);
	}

	.fmt-customize-moves .fmt-move-area {
		flex: 1;
	}

	.fmt-customize-actions {
		border-top: 1px solid var(--border-light);
		padding-top: 9px;
	}

	.fmt-customize-actions .fmt-customize-reset {
		margin-right: auto;
		border-color: transparent;
		background: none;
	}

	.fmt-customize-actions .fmt-customize-done {
		background: var(--accent);
		color: var(--bg-primary);
		border-color: var(--accent);
	}

	.fmt-customize-actions .fmt-customize-done:hover:not(:disabled) {
		background: var(--accent);
		filter: brightness(1.1);
	}

	.fmt-more-panel button:disabled {
		opacity: 0.45;
		cursor: default;
	}

	.fmt-customize-entry:focus-visible,
	.fmt-customize-moves button:focus-visible,
	.fmt-customize-actions button:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: 1px;
	}

	.fmt-customize-error {
		margin: 0;
		font-size: 11px;
		line-height: 1.5;
		color: var(--danger, #e53e3e);
		overflow-wrap: anywhere;
	}


	.fmt-btn:focus-visible,
	.fmt-dropdown button:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: -2px;
	}

	.fmt-btn {
		background: none;
		border: none;
		color: var(--text-secondary);
		cursor: pointer;
		padding: 5px 7px;
		border-radius: 4px;
		font-size: 13px;
		display: flex;
		align-items: center;
		justify-content: center;
		min-width: 30px;
		height: 30px;
		position: relative;
	}

	.fmt-btn :global(svg) {
		width: 18px;
		height: 18px;
	}

	.fmt-btn:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.fmt-btn.active {
		background: var(--accent-light);
		color: var(--text-accent);
	}

	.fmt-sep {
		width: 1px;
		height: 16px;
		background: var(--border-color);
		margin: 0 3px;
		flex-shrink: 0;
	}

	.fmt-dropdown-wrap {
		position: relative;
	}

	.fmt-dropdown {
		position: fixed;
		bottom: auto;
		box-sizing: border-box;
		overflow: auto;
		overscroll-behavior: contain;
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 8px;
		box-shadow: var(--shadow-lg);
		padding: 4px;
		z-index: 100;
		min-width: 140px;
	}

	.fmt-dropdown button {
		display: block;
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

	.fmt-dropdown button:hover {
		background: var(--bg-hover);
	}

	.fmt-dropdown button.active {
		color: var(--text-accent);
		background: var(--accent-light);
	}

	.color-indicator {
		position: absolute;
		bottom: 2px;
		left: 50%;
		transform: translateX(-50%);
		width: 12px;
		height: 2px;
		border-radius: 1px;
	}

	.color-grid-dropdown {
		display: grid;
		grid-template-columns: repeat(4, 1fr);
		gap: 4px;
		padding: 8px;
		min-width: auto;
		width: 140px;
	}

	.color-grid-dropdown .color-swatch {
		width: 28px;
		height: 28px;
		border-radius: 6px;
		border: none;
		cursor: pointer;
		display: flex;
		align-items: center;
		justify-content: center;
		padding: 0;
		min-width: 0;
	}

	.color-grid-dropdown .color-swatch:hover {
		transform: scale(1.15);
	}

	.insert-dropdown {
		min-width: 180px;
	}

	.insert-dropdown button {
		display: flex;
		align-items: center;
		gap: 8px;
	}

	.insert-btn {
		color: var(--accent);
	}

	.align-dropdown {
		min-width: 120px;
	}

	.align-dropdown button {
		display: flex;
		align-items: center;
		gap: 8px;
	}

	.editor-body-wrapper {
		flex: 1;
		display: flex;
		flex-direction: column;
		overflow: hidden;
	}

	.note-search-bar {
		display: flex;
		align-items: center;
		gap: 6px;
		padding: 6px 12px;
		background: var(--bg-secondary);
		border-bottom: 1px solid var(--border);
		flex-shrink: 0;
	}
	.note-search-input {
		flex: 1;
		background: transparent;
		border: none;
		color: var(--text-primary);
		font-size: 13px;
		outline: none;
		min-width: 0;
	}
	.note-search-input::placeholder {
		color: var(--text-secondary);
	}
	.note-search-count {
		font-size: 12px;
		color: var(--text-secondary);
		white-space: nowrap;
	}
	.note-search-btn {
		background: none;
		border: none;
		color: var(--text-secondary);
		cursor: pointer;
		padding: 2px;
		border-radius: 4px;
		display: flex;
		align-items: center;
	}
	.note-search-btn:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}
	.note-search-btn:disabled { opacity: 0.4; cursor: default; }
	.note-search-btn:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
	.note-replace-toggle svg { transition: transform 0.12s ease; }
	.note-replace-toggle.open svg { transform: rotate(90deg); }
	.note-replace-btn { font-size: 12px; height: 24px; padding: 0 8px; }
	.note-search-option { min-width: 24px; justify-content: center; font-size: 12px; height: 24px; }
	.note-search-option[aria-pressed="true"] { background: var(--accent); color: var(--bg-primary); }
	.note-search-option.whole-word { text-decoration: underline; text-underline-offset: 3px; }
	:global(.note-search-match) {
		background: rgba(255, 200, 0, 0.3);
		border-radius: 2px;
	}
	:global(.note-search-active) {
		background: rgba(255, 150, 0, 0.6);
	}

	.editor-body-row {
		flex: 1;
		display: flex;
		flex-direction: row;
		overflow: hidden;
		min-height: 0;
	}

	.editor-body {
		flex: 1;
		overflow-y: auto;
		overflow-anchor: none;
		padding: 8px 20px;
		min-width: 0;
		position: relative;
	}

	.editor-body.history-hidden { display: none; }

	/* Inset the scrollbar off the window's right edge so the window resize handle
	   (ResizeHandles in +layout) has clean space and doesn't swallow the scrollbar. */
	.editor-container:not(.mobile) .editor-body {
		margin-right: 8px;
	}

	.editor-body.source-body {
		overflow: hidden;
	}

	.history-panel {
		width: 220px;
		border-left: 1px solid var(--border-light);
		background: var(--bg-secondary);
		display: flex;
		flex-direction: column;
		overflow: hidden;
		flex-shrink: 0;
	}

	.history-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		flex-shrink: 0;
		padding: 12px 14px;
		border-bottom: 1px solid var(--border-light);
	}

	.history-header h3 {
		font-size: 12px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--text-tertiary);
		margin: 0;
	}

	.history-header-actions {
		display: flex;
		align-items: center;
		gap: 4px;
	}

	.history-create-btn,
	.history-close {
		background: none;
		border: none;
		color: var(--text-tertiary);
		cursor: pointer;
		padding: 2px;
		border-radius: 4px;
		display: flex;
		align-items: center;
	}

	.history-create-btn:hover,
	.history-close:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.history-empty {
		padding: 16px 14px;
		font-size: 12px;
		color: var(--text-tertiary);
		line-height: 1.5;
	}

	.history-list {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
		padding: 4px 6px;
	}


	.history-retry-btn {
		margin-top: 8px;
		padding: 6px 10px;
		border: 1px solid var(--border-color);
		border-radius: 4px;
		background: var(--bg-tertiary);
		color: var(--text-primary);
		cursor: pointer;
	}

	.history-item {
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: 5px;
		width: 100%;
		padding: 8px 10px;
		border: none;
		border-radius: 6px;
		background: none;
		cursor: pointer;
		text-align: left;
		transition: background 0.1s;
	}

	.history-item:hover {
		background: var(--bg-hover);
	}

	.history-item.active {
		background: var(--accent-light);
	}

	.history-date {
		font-size: 12px;
		color: var(--text-primary);
	}

	.history-item.active .history-date {
		color: var(--accent);
		font-weight: 500;
	}

	.history-size {
		font-size: 11px;
		color: var(--text-tertiary);
	}

	.history-picker { display: none; }
	.history-picker label { font-size: 12px; color: var(--text-secondary); }
	.history-picker select {
		width: 100%;
		min-width: 0;
		padding: 8px 10px;
		border: 1px solid var(--border-color);
		border-radius: 6px;
		background: var(--bg-primary);
		color: var(--text-primary);
		font-size: 12px;
	}
	.history-item:focus-visible, .history-picker select:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }

	@media (max-width: 600px) {
		.editor-body-row.reviewing-history { flex-direction: column-reverse; }
		.history-panel { width: 100%; border-left: none; border-bottom: 1px solid var(--border-light); flex: none; }
		.history-header { padding: 8px 12px; }
		.history-header-actions > button { min-width: 44px; min-height: 44px; }
		.history-list { display: none; }
		.history-picker { display: flex; flex-direction: column; gap: 5px; padding: 0 12px 10px; }
		.history-picker select { min-height: 44px; }
	}


	.outline-panel {
		width: 220px;
		border-left: 1px solid var(--border-light);
		background: var(--bg-secondary);
		display: flex;
		flex-direction: column;
		overflow: hidden;
		flex-shrink: 0;
		min-height: 0;
	}

	.outline-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 12px 14px;
		border-bottom: 1px solid var(--border-light);
	}

	.outline-header h3 {
		font-size: 12px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--text-tertiary);
		margin: 0;
	}

	.outline-close {
		background: none;
		border: none;
		color: var(--text-tertiary);
		cursor: pointer;
		padding: 2px;
		border-radius: 4px;
		display: flex;
		align-items: center;
	}

	.outline-close:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.outline-empty {
		padding: 16px 14px;
		font-size: var(--editor-font-size, 14px);
		color: var(--text-tertiary);
		line-height: 1.5;
	}

	.outline-list {
		flex: 1;
		min-height: 0;
		margin: 0;
		list-style: none;
		overflow-y: auto;
		padding: 4px 0;
	}

	.outline-item {
		display: block;
		width: 100%;
		text-align: left;
		background: none;
		border: none;
		padding: 4px 14px;
		font-size: var(--editor-font-size, 14px);
		color: var(--text-secondary);
		cursor: pointer;
		white-space: normal;
		overflow-wrap: anywhere;
		line-height: 1.5;
	}

	.outline-item:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.outline-item.current {
		background: color-mix(in srgb, var(--accent) 12%, var(--bg-secondary));
		color: var(--accent);
		box-shadow: inset 3px 0 var(--accent);
	}

	.outline-level-1 { padding-left: 14px; font-weight: 600; }
	.outline-level-2 { padding-left: 26px; font-weight: 500; }
	.outline-level-3 { padding-left: 38px; }
	.outline-level-4 { padding-left: 50px; }
	.outline-level-5 { padding-left: 62px; }
	.outline-level-6 { padding-left: 74px; }


	.tiptap-wrapper {
		height: 100%;
		user-select: text;
		/* Optional reading-width cap (Settings > Styling > Note Width). Default `none` = full
		   width; when set, the text column is capped and centered. Scrollbar stays at the
		   panel edge because only the inner content is constrained, not .editor-body. (#137) */
		max-width: var(--editor-content-width, none);
		margin-left: auto;
		margin-right: auto;
	}

	:global(.tiptap-wrapper .tiptap) {
		outline: none;
		min-height: 100%;
		user-select: text;
		font-size: var(--editor-font-size, 14px);
		font-family: var(--editor-font-family, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif);
		overflow: hidden;
	}

	.editor-container:not(.mobile) :global(.tiptap-wrapper .tiptap) {
		min-height: calc(100% + var(--editor-scroll-past-end, 65vh));
		padding-bottom: var(--editor-scroll-past-end, 65vh);
	}

	:global(.tiptap-wrapper .tiptap p) {
		margin: 0 0 0.4em;
		line-height: var(--editor-line-height, 1.65);
	}

	:global(.tiptap-wrapper .tiptap h1) {
		font-size: 1.75em;
		font-weight: 700;
		margin: 1.5em 0 0.5em;
		line-height: 1.2;
	}

	:global(.tiptap-wrapper .tiptap h2) {
		font-size: 1.4em;
		font-weight: 600;
		margin: 1.25em 0 0.5em;
		line-height: 1.3;
	}

	:global(.tiptap-wrapper .tiptap h3) {
		font-size: 1.2em;
		font-weight: 600;
		margin: 1em 0 0.5em;
		line-height: 1.3;
	}

	:global(.tiptap-wrapper .tiptap h4) {
		font-size: 1.05em;
		font-weight: 600;
		margin: 0.9em 0 0.4em;
		line-height: 1.35;
	}

	:global(.tiptap-wrapper .tiptap h1:first-child),
	:global(.tiptap-wrapper .tiptap h2:first-child),
	:global(.tiptap-wrapper .tiptap h3:first-child),
	:global(.tiptap-wrapper .tiptap h4:first-child) {
		margin-top: 0;
	}

	:global(.tiptap-wrapper .tiptap strong) {
		font-weight: 600;
	}

	:global(.tiptap-wrapper .tiptap code) {
		background: color-mix(in srgb, var(--accent) 8%, var(--bg-tertiary));
		padding: 2px 6px;
		border-radius: 4px;
		font-family: 'JetBrains Mono', 'Fira Code', monospace;
		font-size: 0.9em;
	}

	:global(.tiptap-wrapper .tiptap :not(pre) > code) { white-space: pre-wrap; }

	:global(.tiptap-wrapper .tiptap pre) {
		background: color-mix(in srgb, var(--accent) 8%, var(--bg-tertiary));
		border-radius: 8px;
		padding: 16px;
		margin: 1em 0;
		position: relative;
	}

	:global(.tiptap-wrapper .tiptap pre code) {
		display: block;
		overflow-x: auto;
		background: none;
		padding: 0;
		font-size: 0.93em;
		line-height: 1.5;
	}

	:global(.tiptap-wrapper .tiptap .code-copy-btn) {
		position: absolute;
		top: 5px;
		right: 5px;
		padding: 2px 6px;
		border-radius: 4px;
		background: transparent;
		border: 1px solid transparent;
		color: var(--text-tertiary);
		cursor: pointer;
		opacity: 0.4;
		z-index: 2;
		display: flex;
		align-items: center;
		justify-content: center;
		line-height: 0;
		transition: opacity 0.15s, background 0.15s, border-color 0.15s, color 0.15s;
	}

	:global(.tiptap-wrapper .tiptap .code-copy-btn svg) {
		width: 1em;
		height: 1em;
	}

	:global(.tiptap-wrapper .tiptap pre:hover .code-copy-btn) {
		opacity: 1;
		background: var(--bg-secondary);
		border-color: var(--border-color);
	}

	:global(.tiptap-wrapper .tiptap .code-copy-btn.copied) {
		opacity: 1;
		color: var(--text-accent);
		background: var(--accent-light);
		border-color: var(--text-accent);
	}

	:global(.tiptap-wrapper .tiptap pre)::after {
		content: attr(data-language);
		position: absolute;
		top: 5px;
		right: 34px;
		padding: 2px 6px;
		border-radius: 4px;
		background: transparent;
		color: var(--text-tertiary);
		font-size: 11px;
		font-family: 'JetBrains Mono', 'Fira Code', monospace;
		cursor: pointer;
		opacity: 0.4;
		pointer-events: none;
		z-index: 1;
		transition: opacity 0.15s;
	}

	:global(.tiptap-wrapper .tiptap pre[data-language=""])::after {
		content: '•••';
	}

	:global(.tiptap-wrapper .tiptap pre:hover)::after {
		opacity: 1;
	}

	:global(.tiptap-wrapper .tiptap .secret-block) {
		border: 1px solid var(--border);
		border-radius: 8px;
		background: color-mix(in srgb, var(--accent) 7%, var(--bg-secondary));
		margin: 1em 0;
		padding: 12px;
		user-select: text;
		box-sizing: border-box;
		max-width: 100%;
	}

	:global(.secret-block-header) {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
		margin-bottom: 10px;
	}

	:global(.secret-block-title) {
		display: inline-flex;
		align-items: center;
		gap: 7px;
		font-weight: 600;
		font-size: 13px;
		color: var(--text-primary);
	}

	:global(.secret-block-lock) {
		flex: 0 0 auto;
	}

	:global(.secret-block-badge) {
		font-family: 'JetBrains Mono', ui-monospace, monospace;
		font-size: 11px;
		color: var(--text-tertiary);
	}

	:global(.secret-block-form),
	:global(.secret-block-actions) {
		display: flex;
		gap: 8px;
		min-width: 0;
	}

	:global(.secret-block-form input) {
		flex: 1;
		min-width: 0;
		width: 0;
		border: 1px solid var(--border);
		border-radius: 5px;
		background: var(--bg-primary);
		color: var(--text-primary);
		padding: 7px 9px;
		font: inherit;
	}

	:global(.secret-block-form input:focus) {
		outline: none;
		border-color: var(--accent);
	}

	:global(.secret-block-form button),
	:global(.secret-block-actions button) {
		border: 1px solid var(--border);
		border-radius: 5px;
		background: var(--bg-primary);
		color: var(--text-primary);
		padding: 7px 10px;
		font: inherit;
		cursor: pointer;
	}

	:global(.secret-block-form button:hover:not(:disabled)),
	:global(.secret-block-actions button:hover) {
		border-color: var(--accent);
	}

	:global(.secret-block-form button:disabled) {
		opacity: 0.6;
		cursor: not-allowed;
	}

	:global(.secret-block-error) {
		color: #d32f2f;
		font-size: 12px;
		margin-top: 8px;
	}

	:global(.secret-block-plaintext) {
		margin: 0;
		white-space: pre-wrap;
		word-break: break-word;
		font-family: 'JetBrains Mono', ui-monospace, monospace;
		font-size: 13px;
		line-height: 1.5;
		color: var(--text-primary);
	}

	/* Syntax highlighting - light mode */
	:global(.tiptap pre code .hljs-keyword),
	:global(.tiptap pre code .hljs-selector-tag),
	:global(.tiptap pre code .hljs-built_in) { color: #d73a49; }
	:global(.tiptap pre code .hljs-string),
	:global(.tiptap pre code .hljs-addition) { color: #032f62; }
	:global(.tiptap pre code .hljs-number),
	:global(.tiptap pre code .hljs-literal) { color: #005cc5; }
	:global(.tiptap pre code .hljs-comment),
	:global(.tiptap pre code .hljs-quote) { color: #6a737d; font-style: italic; }
	:global(.tiptap pre code .hljs-function),
	:global(.tiptap pre code .hljs-title) { color: #6f42c1; }
	:global(.tiptap pre code .hljs-type),
	:global(.tiptap pre code .hljs-title.class_) { color: #e36209; }
	:global(.tiptap pre code .hljs-variable),
	:global(.tiptap pre code .hljs-template-variable) { color: #e36209; }
	:global(.tiptap pre code .hljs-attr),
	:global(.tiptap pre code .hljs-attribute) { color: #005cc5; }
	:global(.tiptap pre code .hljs-tag) { color: #22863a; }
	:global(.tiptap pre code .hljs-name) { color: #22863a; }
	:global(.tiptap pre code .hljs-meta) { color: #005cc5; }
	:global(.tiptap pre code .hljs-deletion) { color: #b31d28; background: #ffeef0; }
	:global(.tiptap pre code .hljs-symbol),
	:global(.tiptap pre code .hljs-bullet) { color: #005cc5; }
	:global(.tiptap pre code .hljs-regexp) { color: #032f62; }
	:global(.tiptap pre code .hljs-params) { color: #24292e; }
	:global(.tiptap pre code .hljs-punctuation) { color: #24292e; }
	:global(.tiptap pre code .hljs-property) { color: #005cc5; }
	:global(.tiptap pre code .hljs-selector-class) { color: #6f42c1; }
	:global(.tiptap pre code .hljs-selector-id) { color: #005cc5; }
	:global(.tiptap pre code .hljs-operator) { color: #d73a49; }

	/* Syntax highlighting - dark mode */
	:global(.dark .tiptap pre code .hljs-keyword),
	:global(.dark .tiptap pre code .hljs-selector-tag),
	:global(.dark .tiptap pre code .hljs-built_in) { color: #ff7b72; }
	:global(.dark .tiptap pre code .hljs-string),
	:global(.dark .tiptap pre code .hljs-addition) { color: #a5d6ff; }
	:global(.dark .tiptap pre code .hljs-number),
	:global(.dark .tiptap pre code .hljs-literal) { color: #79c0ff; }
	:global(.dark .tiptap pre code .hljs-comment),
	:global(.dark .tiptap pre code .hljs-quote) { color: #8b949e; font-style: italic; }
	:global(.dark .tiptap pre code .hljs-function),
	:global(.dark .tiptap pre code .hljs-title) { color: #d2a8ff; }
	:global(.dark .tiptap pre code .hljs-type),
	:global(.dark .tiptap pre code .hljs-title.class_) { color: #ffa657; }
	:global(.dark .tiptap pre code .hljs-variable),
	:global(.dark .tiptap pre code .hljs-template-variable) { color: #ffa657; }
	:global(.dark .tiptap pre code .hljs-attr),
	:global(.dark .tiptap pre code .hljs-attribute) { color: #79c0ff; }
	:global(.dark .tiptap pre code .hljs-tag) { color: #7ee787; }
	:global(.dark .tiptap pre code .hljs-name) { color: #7ee787; }
	:global(.dark .tiptap pre code .hljs-meta) { color: #79c0ff; }
	:global(.dark .tiptap pre code .hljs-deletion) { color: #ffdcd7; background: #67060c; }
	:global(.dark .tiptap pre code .hljs-symbol),
	:global(.dark .tiptap pre code .hljs-bullet) { color: #79c0ff; }
	:global(.dark .tiptap pre code .hljs-regexp) { color: #a5d6ff; }
	:global(.dark .tiptap pre code .hljs-params) { color: #c9d1d9; }
	:global(.dark .tiptap pre code .hljs-punctuation) { color: #c9d1d9; }
	:global(.dark .tiptap pre code .hljs-property) { color: #79c0ff; }
	:global(.dark .tiptap pre code .hljs-selector-class) { color: #d2a8ff; }
	:global(.dark .tiptap pre code .hljs-selector-id) { color: #79c0ff; }
	:global(.dark .tiptap pre code .hljs-operator) { color: #ff7b72; }

	.math-modal-overlay {
		position: fixed;
		inset: 0;
		background: rgba(0, 0, 0, 0.4);
		z-index: 2200;
		display: flex;
		align-items: center;
		justify-content: center;
	}
	.math-modal {
		background: var(--bg-primary);
		border: 1px solid var(--border);
		border-radius: 8px;
		min-width: min(480px, 92vw);
		max-width: 80vw;
		display: flex;
		flex-direction: column;
		gap: 12px;
		padding: 16px;
		box-shadow: 0 10px 40px rgba(0, 0, 0, 0.3);
	}
	.math-modal-header {
		display: flex;
		justify-content: space-between;
		align-items: center;
		font-weight: 500;
		font-size: 14px;
		color: var(--text-primary);
	}
	.math-modal-close {
		background: none;
		border: none;
		cursor: pointer;
		color: var(--text-secondary);
		padding: 4px;
		display: flex;
	}
	.math-modal-close:hover { color: var(--text-primary); }
	.math-modal-input {
		font-family: var(--font-mono, ui-monospace, monospace);
		font-size: 13px;
		min-height: 80px;
		padding: 10px;
		border: 1px solid var(--border);
		border-radius: 5px;
		background: var(--bg-secondary);
		color: var(--text-primary);
		resize: vertical;
		outline: none;
	}
	.math-modal-input:focus { border-color: var(--accent); }
	.img-title-input {
		min-height: 0;
		resize: none;
		font-family: inherit;
	}
	.math-modal-preview {
		min-height: 60px;
		padding: 14px;
		border: 1px solid var(--border);
		border-radius: 5px;
		background: var(--bg-secondary);
		display: flex;
		align-items: center;
		justify-content: center;
		overflow-x: auto;
		color: var(--text-primary);
	}
	.math-modal-preview-empty {
		color: var(--text-tertiary);
		font-size: 13px;
	}
	.math-modal-preview :global(.math-modal-preview-error) {
		color: #d32f2f;
		font-size: 12px;
		font-family: var(--font-mono, monospace);
	}
	.math-modal-footer {
		display: flex;
		justify-content: space-between;
		align-items: center;
		gap: 12px;
	}
	.math-modal-hint {
		font-size: 11px;
		color: var(--text-tertiary);
	}
	.math-modal-actions {
		display: flex;
		gap: 8px;
	}
	.math-modal-actions button {
		appearance: none;
		padding: 6px 14px;
		border-radius: 5px;
		cursor: pointer;
		font-size: 13px;
		font-family: inherit;
		border: 1px solid var(--border);
		background: var(--bg-secondary);
		color: var(--text-primary);
	}
	.math-modal-actions button:hover:not(:disabled) {
		border-color: var(--accent);
	}
	.math-modal-actions button.primary {
		background: var(--accent);
		color: var(--accent-fg, white);
		border-color: var(--accent);
	}
	.math-modal-actions button.primary:hover:not(:disabled) {
		filter: brightness(1.1);
	}
	.math-modal-actions button:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}

	.secret-modal {
		max-width: min(720px, 92vw);
	}

	.secret-modal-text {
		min-height: 140px;
	}

	.secret-modal-title {
		border: 1px solid var(--border);
		border-radius: 5px;
		background: var(--bg-secondary);
		color: var(--text-primary);
		padding: 9px 10px;
		font: inherit;
		outline: none;
	}

	.secret-modal-title:focus {
		border-color: var(--accent);
	}

	.secret-modal-fields {
		display: grid;
		grid-template-columns: 1fr 1fr;
		gap: 10px;
	}

	.secret-modal-fields input {
		border: 1px solid var(--border);
		border-radius: 5px;
		background: var(--bg-secondary);
		color: var(--text-primary);
		padding: 9px 10px;
		font: inherit;
		outline: none;
	}

	.secret-modal-fields input:focus {
		border-color: var(--accent);
	}

	.secret-modal-error {
		color: #d32f2f;
		font-size: 12px;
	}

	@media (max-width: 640px) {
		.secret-modal-fields {
			grid-template-columns: 1fr;
		}
	}

	.viewer-banner {
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 8px 14px;
		background: var(--bg-tertiary, var(--bg-secondary));
		border-bottom: 1px solid var(--border);
		font-size: 13px;
		color: var(--text-primary);
		flex-shrink: 0;
	}
	.viewer-banner-icon {
		flex-shrink: 0;
		color: var(--accent, var(--text-secondary));
	}
	.viewer-banner-label {
		font-weight: 500;
		flex-shrink: 0;
	}
	.viewer-banner-path {
		color: var(--text-secondary);
		font-family: var(--font-mono, monospace);
		font-size: 12px;
		overflow: hidden;
		white-space: nowrap;
		text-overflow: ellipsis;
		flex: 1;
		min-width: 0;
	}
	.viewer-banner-actions {
		display: flex;
		gap: 6px;
		flex-shrink: 0;
	}
	.viewer-banner-btn {
		appearance: none;
		background: var(--bg-primary);
		color: var(--text-primary);
		border: 1px solid var(--border);
		border-radius: 5px;
		padding: 4px 12px;
		font-size: 12px;
		cursor: pointer;
		font-family: inherit;
	}
	.viewer-banner-btn:hover:not(:disabled) {
		border-color: var(--accent);
	}
	.viewer-banner-btn:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}
	.viewer-banner-btn.primary {
		background: var(--accent);
		color: var(--accent-fg, white);
		border-color: var(--accent);
	}
	.viewer-banner-btn.primary:hover:not(:disabled) {
		filter: brightness(1.1);
	}
	.viewer-banner-toast {
		font-size: 12px;
		color: var(--text-secondary);
		flex-shrink: 0;
	}

	.viewer-import-overlay {
		position: fixed;
		inset: 0;
		background: rgba(0, 0, 0, 0.4);
		z-index: 2100;
		display: flex;
		align-items: center;
		justify-content: center;
	}
	.viewer-import-picker {
		background: var(--bg-primary);
		border: 1px solid var(--border);
		border-radius: 8px;
		min-width: 320px;
		max-width: 480px;
		max-height: 70vh;
		display: flex;
		flex-direction: column;
		overflow: hidden;
		box-shadow: 0 10px 40px rgba(0, 0, 0, 0.3);
	}
	.viewer-import-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 12px 16px;
		border-bottom: 1px solid var(--border);
		font-weight: 500;
		font-size: 14px;
	}
	.viewer-import-close {
		appearance: none;
		background: none;
		border: none;
		color: var(--text-secondary);
		cursor: pointer;
		padding: 4px;
		display: flex;
	}
	.viewer-import-close:hover {
		color: var(--text-primary);
	}
	.viewer-import-list {
		overflow-y: auto;
		padding: 6px 0;
	}
	.viewer-import-item {
		display: flex;
		align-items: center;
		gap: 8px;
		width: 100%;
		appearance: none;
		background: none;
		border: none;
		color: var(--text-primary);
		padding: 8px 14px;
		text-align: left;
		font-size: 13px;
		cursor: pointer;
		font-family: inherit;
	}
	.viewer-import-item:hover:not(:disabled) {
		background: var(--bg-secondary);
	}
	.viewer-import-item:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}
	.viewer-import-item svg {
		color: var(--text-tertiary);
		flex-shrink: 0;
	}

	.tag-menu-overlay {
		position: fixed;
		inset: 0;
		z-index: 2000;
	}

	.tag-menu {
		position: fixed;
		width: 240px;
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 8px;
		box-shadow: var(--shadow-lg);
		padding: 8px;
		z-index: 2001;
	}

	.tag-menu-list {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
		margin-bottom: 8px;
	}

	.tag-menu-chip {
		display: inline-flex;
		align-items: center;
		gap: 3px;
		font-size: 11px;
		color: var(--text-secondary);
		background: var(--bg-tertiary);
		padding: 2px 4px 2px 8px;
		border-radius: 10px;
	}

	.tag-menu-remove {
		display: flex;
		align-items: center;
		justify-content: center;
		border: none;
		background: none;
		color: var(--text-tertiary);
		cursor: pointer;
		padding: 1px;
		border-radius: 50%;
	}

	.tag-menu-remove:hover {
		color: var(--danger, #e53e3e);
		background: color-mix(in srgb, var(--danger, #e53e3e) 12%, transparent);
	}

	.code-lang-overlay {
		position: fixed;
		inset: 0;
		z-index: 2000;
	}

	.code-lang-dropdown {
		position: fixed;
		transform: translateX(-100%);
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 8px;
		box-shadow: var(--shadow-lg);
		padding: 4px;
		max-height: 320px;
		overflow-y: auto;
		min-width: 150px;
		z-index: 2001;
	}

	.code-lang-search {
		width: 100%;
		box-sizing: border-box;
		padding: 5px 8px;
		margin-bottom: 4px;
		border: 1px solid var(--border-color);
		border-radius: 5px;
		background: var(--bg-secondary);
		color: var(--text-primary);
		font-size: 12px;
		outline: none;
	}

	.code-lang-search:focus {
		border-color: var(--accent);
	}

	.code-lang-empty {
		padding: 8px;
		text-align: center;
		color: var(--text-tertiary);
		font-size: 12px;
	}

	.code-lang-dropdown::-webkit-scrollbar {
		width: 5px;
	}

	.code-lang-dropdown::-webkit-scrollbar-track {
		background: transparent;
	}

	.code-lang-dropdown::-webkit-scrollbar-thumb {
		background: var(--border-color);
		border-radius: 3px;
	}

	.code-lang-option {
		display: block;
		width: 100%;
		padding: 5px 10px;
		border: none;
		background: none;
		color: var(--text-primary);
		font-size: 12px;
		font-family: 'JetBrains Mono', 'Fira Code', monospace;
		cursor: pointer;
		border-radius: 4px;
		text-align: left;
	}

	.code-lang-option:hover {
		background: var(--bg-hover);
	}

	.code-lang-option.active {
		color: var(--text-accent);
		background: var(--accent-light);
	}

	:global(.tiptap-wrapper .tiptap blockquote) {
		border-left: 3px solid var(--accent);
		padding-left: 16px;
		margin: 1em 0;
		color: var(--text-secondary);
	}

	/* Callouts (Obsidian-style) */
	:global(.tiptap-wrapper .tiptap .callout) {
		--cc: var(--callout-note);
		margin: 1em 0;
		border: 1px solid rgba(var(--cc), 0.3);
		border-left: 3px solid rgb(var(--cc));
		border-radius: 8px;
		background: rgba(var(--cc), 0.06);
		overflow: hidden;
	}
	:global(.tiptap-wrapper .tiptap .callout[data-callout-group="note"]) { --cc: var(--callout-note); }
	:global(.tiptap-wrapper .tiptap .callout[data-callout-group="abstract"]) { --cc: var(--callout-abstract); }
	:global(.tiptap-wrapper .tiptap .callout[data-callout-group="info"]) { --cc: var(--callout-info); }
	:global(.tiptap-wrapper .tiptap .callout[data-callout-group="todo"]) { --cc: var(--callout-todo); }
	:global(.tiptap-wrapper .tiptap .callout[data-callout-group="tip"]) { --cc: var(--callout-tip); }
	:global(.tiptap-wrapper .tiptap .callout[data-callout-group="success"]) { --cc: var(--callout-success); }
	:global(.tiptap-wrapper .tiptap .callout[data-callout-group="question"]) { --cc: var(--callout-question); }
	:global(.tiptap-wrapper .tiptap .callout[data-callout-group="warning"]) { --cc: var(--callout-warning); }
	:global(.tiptap-wrapper .tiptap .callout[data-callout-group="failure"]) { --cc: var(--callout-failure); }
	:global(.tiptap-wrapper .tiptap .callout[data-callout-group="danger"]) { --cc: var(--callout-danger); }
	:global(.tiptap-wrapper .tiptap .callout[data-callout-group="bug"]) { --cc: var(--callout-bug); }
	:global(.tiptap-wrapper .tiptap .callout[data-callout-group="example"]) { --cc: var(--callout-example); }
	:global(.tiptap-wrapper .tiptap .callout[data-callout-group="quote"]) { --cc: var(--callout-quote); }
	:global(.tiptap-wrapper .tiptap .callout[data-callout-group="custom"]) { --cc: var(--callout-custom); }

	:global(.tiptap-wrapper .tiptap .callout-header) {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 8px 12px;
		font-weight: 600;
		color: rgb(var(--cc));
		background: rgba(var(--cc), 0.1);
		user-select: none;
	}
	:global(.tiptap-wrapper .tiptap .callout-icon),
	:global(.tiptap-wrapper .tiptap .callout-fold) {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 22px;
		height: 22px;
		flex: 0 0 auto;
		padding: 0;
		border: none;
		background: none;
		cursor: pointer;
		color: rgb(var(--cc));
		border-radius: 4px;
		transition: background 0.15s, transform 0.2s;
	}
	:global(.tiptap-wrapper .tiptap .callout-icon:hover),
	:global(.tiptap-wrapper .tiptap .callout-fold:hover) {
		background: rgba(var(--cc), 0.18);
	}
	:global(.tiptap-wrapper .tiptap .callout.is-folded .callout-fold) {
		transform: rotate(-90deg);
	}
	:global(.tiptap-wrapper .tiptap .callout-title) {
		flex: 1 1 auto;
		min-width: 0;
		border: none;
		background: none;
		outline: none;
		font: inherit;
		font-weight: 600;
		color: rgb(var(--cc));
		padding: 2px 0;
	}
	:global(.tiptap-wrapper .tiptap .callout-title::placeholder) {
		color: rgb(var(--cc));
		opacity: 0.7;
	}
	:global(.tiptap-wrapper .tiptap .callout-content) {
		padding: 8px 14px 10px;
	}
	:global(.tiptap-wrapper .tiptap .callout.is-folded .callout-content) {
		display: none;
	}
	:global(.tiptap-wrapper .tiptap .callout-content > p:first-child) { margin-top: 0; }
	:global(.tiptap-wrapper .tiptap .callout-content > p:last-child) { margin-bottom: 0; }

	/* Callout type picker (rendered into <body>) */
	:global(.callout-type-menu) {
		position: fixed;
		z-index: 9999;
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 8px;
		box-shadow: var(--shadow-lg);
		padding: 4px;
		display: grid;
		grid-template-columns: 1fr 1fr;
		gap: 2px;
		max-height: 60vh;
		overflow: auto;
	}
	:global(.callout-type-option) {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 6px 10px;
		border: none;
		background: none;
		border-radius: 5px;
		cursor: pointer;
		font: inherit;
		font-size: 13px;
		color: var(--text-primary);
		text-align: left;
		white-space: nowrap;
	}
	:global(.callout-type-option:hover) {
		background: var(--bg-hover);
	}
	:global(.callout-type-icon) {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 18px;
		height: 18px;
		flex: 0 0 auto;
	}
	:global(.callout-type-menu.is-custom) {
		display: block;
	}
	:global(.callout-type-custom-input) {
		width: 200px;
		max-width: 60vw;
		border: 1px solid var(--border-color);
		border-radius: 5px;
		background: var(--bg-secondary);
		color: var(--text-primary);
		padding: 7px 9px;
		font: inherit;
		font-size: 13px;
		outline: none;
	}
	:global(.callout-type-custom-input:focus) {
		border-color: var(--accent);
	}

	:global(.tiptap-wrapper .tiptap [data-type="details"]) {
		margin: 1em 0;
		border: 1px solid var(--border-color);
		border-radius: 8px;
		position: relative;
	}

	:global(.tiptap-wrapper .tiptap [data-type="details"] > button) {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 24px;
		height: 24px;
		position: absolute;
		top: 8px;
		left: 6px;
		background: none;
		border: none;
		cursor: pointer;
		color: var(--text-secondary);
		padding: 0;
		border-radius: 4px;
		transition: background 0.15s;
	}

	:global(.tiptap-wrapper .tiptap [data-type="details"] > button:hover) {
		background: var(--bg-tertiary);
	}

	:global(.tiptap-wrapper .tiptap [data-type="details"] > button::after) {
		content: '▶';
		font-size: 10px;
		transition: transform 0.2s;
	}

	:global(.tiptap-wrapper .tiptap [data-type="details"].is-open > button::after) {
		transform: rotate(90deg);
	}

	@starting-style {
		:global(.tiptap-wrapper .tiptap [data-type="details"].is-open > button::after) {
			transform: rotate(0deg);
		}
	}

	:global(.tiptap-wrapper .tiptap [data-type="details"] summary) {
		padding: 10px 14px 10px 32px;
		font-weight: 600;
		background: var(--bg-secondary);
		transition: background 0.15s;
		list-style: none;
	}

	:global(.tiptap-wrapper .tiptap [data-type="details"] summary::-webkit-details-marker) {
		display: none;
	}

	:global(.tiptap-wrapper .tiptap [data-type="details"] summary:hover) {
		background: var(--bg-tertiary);
	}

	:global(.tiptap-wrapper .tiptap [data-type="details"] [data-type="detailsContent"]) {
		padding: 10px 14px;
	}

	:global(.tiptap-wrapper .tiptap [data-type="details"] [data-type="detailsContent"] > p:first-child) {
		margin-top: 0;
	}

	:global(.tiptap-wrapper .tiptap [data-type="details"] [data-type="detailsContent"] > p:last-child) {
		margin-bottom: 0;
	}

	:global(.tiptap-wrapper .tiptap ul),
	:global(.tiptap-wrapper .tiptap ol) {
		padding-left: 24px;
		margin: 0.5em 0;
	}

	:global(.tiptap-wrapper .tiptap ul:not([data-type="taskList"])) {
		list-style-type: disc;
	}

	:global(.tiptap-wrapper .tiptap ul:not([data-type="taskList"]) ul) {
		list-style-type: circle;
	}

	:global(.tiptap-wrapper .tiptap ul:not([data-type="taskList"]) ul ul) {
		list-style-type: square;
	}

	/* Counter-based numbering: WebKitGTK clips the native ol marker (overflow:hidden), so 11 rendered as 1. list-item respects <ol start> + nesting. */
	:global(.tiptap-wrapper .tiptap ol) {
		list-style: none;
		padding-left: 0;
	}
	:global(.tiptap-wrapper .tiptap ol > li) {
		position: relative;
		padding-left: 2.4em;
	}
	:global(.tiptap-wrapper .tiptap ol > li::before) {
		content: counter(list-item) ".";
		position: absolute;
		left: 0;
		width: 2em;
		text-align: right;
		white-space: nowrap;
	}
	:global(.tiptap-wrapper .tiptap ol ol > li::before) {
		content: counter(list-item, lower-alpha) ".";
	}
	:global(.tiptap-wrapper .tiptap ol ol ol > li::before) {
		content: counter(list-item, lower-roman) ".";
	}

	:global(.tiptap-wrapper .tiptap li) {
		margin: 0.25em 0;
		line-height: var(--editor-line-height, 1.65);
	}

	:global(.tiptap-wrapper .tiptap li p) {
		margin: 0;
	}

	:global(.tiptap-wrapper .tiptap ul[data-type="taskList"]) {
		list-style: none;
		padding-left: 0;
	}

	:global(.tiptap-wrapper .tiptap ul[data-type="taskList"] li) {
		display: flex;
		align-items: flex-start;
		gap: 8px;
		margin: 4px 0;
	}

	:global(.tiptap-wrapper .tiptap ul[data-type="taskList"] li.task-target-reveal) {
		border-radius: 6px;
		animation: task-target-flash 2.2s ease-out;
	}

	@keyframes task-target-flash {
		0%, 62% {
			background: color-mix(in srgb, var(--accent) 24%, var(--bg-editor));
		}
		100% {
			background: transparent;
		}
	}


	@media (prefers-reduced-motion: reduce) {
		:global(.tiptap-wrapper .tiptap ul[data-type="taskList"] li.task-target-reveal) {
			animation: none;
			background: color-mix(in srgb, var(--accent) 24%, var(--bg-editor));
		}
	}

	:global(.tiptap-wrapper .tiptap ul[data-type="taskList"] li label) {
		display: flex;
		align-items: center;
		margin-top: 4px;
		flex-shrink: 0;
	}

	:global(.tiptap-wrapper .tiptap ul[data-type="taskList"] li label input[type="checkbox"]) {
		appearance: none;
		-webkit-appearance: none;
		width: 16px;
		height: 16px;
		border: 2px solid var(--border-color);
		border-radius: 4px;
		cursor: pointer;
		position: relative;
		flex-shrink: 0;
		background: var(--bg-primary);
		transition: background 0.15s, border-color 0.15s;
	}

	:global(.tiptap-wrapper .tiptap ul[data-type="taskList"] li[data-checked="true"] label input[type="checkbox"]) {
		background: var(--accent);
		border-color: var(--accent);
	}

	:global(.tiptap-wrapper .tiptap ul[data-type="taskList"] li[data-checked="true"] label input[type="checkbox"]::after) {
		content: '';
		position: absolute;
		left: 3px;
		top: 0px;
		width: 6px;
		height: 10px;
		border: solid white;
		border-width: 0 2px 2px 0;
		transform: rotate(45deg);
	}

	:global(.tiptap-wrapper .tiptap ul[data-type="taskList"] li label input[type="checkbox"]:hover) {
		border-color: var(--accent);
	}

	:global(.tiptap-wrapper .tiptap ul[data-type="taskList"] li > div) {
		flex: 1;
		min-width: 0;
	}

	/* Strike through only the direct paragraph content of a checked task item.
	   Using > p instead of > div prevents text-decoration from bleeding into
	   nested task lists, since CSS text-decoration cannot be cancelled by descendants. */
	:global(.tiptap-wrapper .tiptap ul[data-type="taskList"] li[data-checked="true"] > div > p) {
		text-decoration: line-through;
		color: var(--text-tertiary);
	}

	/* Dim inline task metadata (!high / due:YYYY-MM-DD) so it recedes but stays readable. */
	:global(.tiptap-wrapper .tiptap .task-meta-dim) {
		opacity: 0.45;
	}

	:global(.tiptap-wrapper .tiptap hr) {
		border: none;
		border-top: 1px solid var(--border-color);
		margin: 1.5em 0;
	}

	:global(.tiptap-wrapper .tiptap a) {
		color: var(--text-accent);
		text-decoration: underline;
		text-decoration-color: color-mix(in srgb, var(--text-accent) 40%, transparent);
	}

	:global(.tiptap-wrapper .tiptap a::after) {
		content: '↗';
		display: inline;
		font-size: 0.65em;
		margin-left: 2px;
		opacity: 0.5;
		vertical-align: 15%;
	}

	:global(html.no-link-arrows .tiptap-wrapper .tiptap a::after) {
		content: none;
	}

	:global(html.no-link-arrows .tiptap-wrapper .tiptap a[href$=".md"]::after) {
		content: none;
	}

	:global(.tiptap-wrapper .tiptap a[href$=".md"]::after) {
		content: '⤴';
	}

	:global(.tiptap-wrapper .tiptap a:hover) {
		text-decoration-color: var(--text-accent);
	}

	:global(.tiptap-wrapper .tiptap a:hover::after) {
		opacity: 0.8;
	}

	:global(.tiptap-wrapper .tiptap img) {
		display: block;
		max-width: 100%;
		height: auto;
		border-radius: 8px;
		margin: 1em 0;
		cursor: pointer;
	}

	:global(.tiptap-wrapper .tiptap img:hover) {
		outline: none;
	}

	:global(.tiptap-wrapper .tiptap img.ProseMirror-selectednode) {
		outline: none !important;
		box-shadow: none !important;
		background: none !important;
	}

	:global(.tiptap-wrapper .tiptap .ProseMirror-selectednode) {
		outline: none !important;
	}

	:global(.tiptap-wrapper .tiptap img::selection) {
		background: transparent;
	}

	:global(.tiptap-wrapper .tiptap img::-moz-selection) {
		background: transparent;
	}

	:global(.tiptap-wrapper .tiptap img[data-size="small"]) {
		max-width: 33%;
	}

	:global(.tiptap-wrapper .tiptap img[data-size="medium"]) {
		max-width: 65%;
	}

	:global(.tiptap-wrapper .tiptap img[data-size="full"]) {
		max-width: 100%;
	}

	/* Image grids (#254). These come after the size rules above so grid cells can override them. */
	:global(.tiptap-wrapper .tiptap .img-grid-wrap) {
		margin: 1em 0;
	}
	:global(.tiptap-wrapper .tiptap .img-grid-header) {
		display: none;
		gap: 4px;
		margin-bottom: 6px;
	}
	:global(.tiptap-wrapper .tiptap[contenteditable="true"] .img-grid-header) {
		display: flex;
	}
	:global(.tiptap-wrapper .tiptap .img-grid-control) {
		font-size: 11px;
		font-weight: 600;
		padding: 2px 8px;
		border-radius: 6px;
		border: 1px solid var(--border-color);
		background: var(--bg-secondary);
		color: var(--text-secondary);
		cursor: pointer;
	}
	:global(.tiptap-wrapper .tiptap .img-grid) {
		display: grid;
		grid-template-columns: repeat(2, 1fr);
		gap: 8px;
	}
	:global(.tiptap-wrapper .tiptap .img-grid[data-cols="3"]) {
		grid-template-columns: repeat(3, 1fr);
	}
	:global(.tiptap-wrapper .tiptap .img-grid[data-cols="4"]) {
		grid-template-columns: repeat(4, 1fr);
	}
	:global(.tiptap-wrapper .tiptap .img-grid[data-layout="masonry"]) {
		display: block;
		columns: 2;
		column-gap: 8px;
	}
	:global(.tiptap-wrapper .tiptap .img-grid[data-layout="masonry"][data-cols="3"]) {
		columns: 3;
	}
	:global(.tiptap-wrapper .tiptap .img-grid[data-layout="masonry"][data-cols="4"]) {
		columns: 4;
	}
	:global(.tiptap-wrapper .tiptap .img-grid img) {
		margin: 0;
		width: 100%;
		max-width: 100%;
	}
	/* S/M/L still applies inside a cell. */
	:global(.tiptap-wrapper .tiptap .img-grid img[data-size="small"]) {
		width: 55%;
		justify-self: center;
	}
	:global(.tiptap-wrapper .tiptap .img-grid img[data-size="medium"]) {
		width: 80%;
		justify-self: center;
	}
	:global(.tiptap-wrapper .tiptap .img-grid[data-layout="masonry"] img) {
		display: block;
		break-inside: avoid;
		margin-bottom: 8px;
	}
	:global(.tiptap-wrapper .tiptap .img-grid[data-layout="masonry"] img[data-size="small"]),
	:global(.tiptap-wrapper .tiptap .img-grid[data-layout="masonry"] img[data-size="medium"]) {
		margin-left: auto;
		margin-right: auto;
	}
	:global(.tiptap-wrapper .tiptap[contenteditable="true"] .img-grid img) {
		cursor: grab;
	}
	:global(.tiptap-wrapper .tiptap .img-grid.img-grid-dropping) {
		outline: 2px dashed var(--accent);
		outline-offset: 4px;
	}
	/* Phones: 3 and 4 column grids collapse to 2. */
	@media (max-width: 640px) {
		:global(.tiptap-wrapper .tiptap .img-grid[data-cols="3"]),
		:global(.tiptap-wrapper .tiptap .img-grid[data-cols="4"]) {
			grid-template-columns: repeat(2, 1fr);
		}
		:global(.tiptap-wrapper .tiptap .img-grid[data-layout="masonry"][data-cols="3"]),
		:global(.tiptap-wrapper .tiptap .img-grid[data-layout="masonry"][data-cols="4"]) {
			columns: 2;
		}
	}

	.img-toolbar-overlay {
		position: fixed;
		inset: 0;
		z-index: 1500;
	}

	.img-toolbar {
		position: fixed;
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		max-width: calc(100vw - 16px);
		gap: 2px;
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 8px;
		box-shadow: var(--shadow-lg);
		padding: 3px;
		z-index: 1501;
	}

	.img-toolbar button {
		padding: 5px 14px;
		border: none;
		background: none;
		color: var(--text-secondary);
		font-size: 12px;
		font-weight: 600;
		cursor: pointer;
		border-radius: 5px;
	}

	.img-toolbar button:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.img-toolbar button.active {
		background: var(--accent);
		color: white;
	}

	.img-toolbar-sep {
		width: 1px;
		height: 16px;
		background: var(--border-color);
		margin: 0 2px;
	}

	.img-toolbar button svg {
		display: block;
	}

	.img-toolbar.mobile-viewer-toolbar button {
		min-width: 40px;
		min-height: 40px;
		padding: 8px 12px;
	}

	.copy-toast {
		position: fixed;
		bottom: 24px;
		right: 24px;
		display: flex;
		align-items: center;
		gap: 6px;
		padding: 8px 16px;
		min-width: 100px;
		justify-content: center;
		background: var(--accent);
		border-radius: 8px;
		box-shadow: 0 4px 16px rgba(0, 0, 0, 0.3);
		font-size: 13px;
		font-weight: 500;
		color: white;
		z-index: 9999;
		animation: toast-in 0.15s ease-out;
	}

	.copy-toast.done {
		background: var(--accent);
	}

	.copy-toast-spinner {
		animation: copy-spin 0.8s linear infinite;
	}

	@keyframes toast-in {
		from { opacity: 0; transform: translateY(8px); }
		to { opacity: 1; transform: translateY(0); }
	}

	@keyframes copy-spin {
		to { transform: rotate(360deg); }
	}

	:global(.tiptap-wrapper .tiptap mark) {
		padding: 0px 5px 2px;
		border-radius: 3px;
		color: #333 !important;
		box-decoration-break: clone;
		-webkit-box-decoration-break: clone;
	}

	:global(.dark .tiptap-wrapper .tiptap mark) {
		color: #eee !important;
	}

	:global(.tiptap-wrapper .tiptap .tableWrapper) {
		overflow-x: auto;
		margin: 1em 0;
	}

	:global(.tiptap-wrapper .tiptap table) {
		border-collapse: collapse;
		width: auto;
		table-layout: fixed;
		overflow: hidden;
	}

	:global(.tiptap-wrapper .tiptap th),
	:global(.tiptap-wrapper .tiptap td) {
		border: 1px solid var(--border-color);
		padding: 8px 12px;
		text-align: left;
		vertical-align: top;
		position: relative;
		min-width: 80px;
		box-sizing: border-box;
	}

	:global(.tiptap-wrapper .tiptap th) {
		background: var(--bg-tertiary);
		font-weight: 600;
	}

	:global(.tiptap-wrapper .tiptap td > p),
	:global(.tiptap-wrapper .tiptap th > p) {
		margin: 0;
	}

	:global(.tiptap-wrapper .tiptap .selectedCell::after) {
		content: "";
		position: absolute;
		inset: 0;
		background: var(--accent-light);
		pointer-events: none;
		z-index: 1;
	}

	:global(.tiptap-wrapper .tiptap .column-resize-handle) {
		position: absolute;
		right: -2px;
		top: 0;
		bottom: -2px;
		width: 4px;
		background: var(--accent);
		pointer-events: none;
		z-index: 2;
	}

	:global(.tiptap-wrapper .tiptap.resize-cursor) {
		cursor: col-resize;
	}

	:global(.tiptap-wrapper .tiptap > .is-empty::before) {
		content: attr(data-placeholder);
		color: var(--text-tertiary);
		pointer-events: none;
		float: left;
		height: 0;
		padding-left: 2px;
	}

	/* Suppress placeholder on task list / details / callout containers - it overlaps their own UI (checkbox, toggle, callout header) */
	:global(.tiptap-wrapper .tiptap > ul[data-type="taskList"].is-empty::before),
	:global(.tiptap-wrapper .tiptap > [data-type="details"].is-empty::before),
	:global(.tiptap-wrapper .tiptap > .callout.is-empty::before) {
		content: none;
	}

	/* Show placeholder on the paragraph inside task item content div */
	:global(.tiptap-wrapper .tiptap ul[data-type="taskList"] li > div > p.is-empty::before) {
		content: attr(data-placeholder);
		color: var(--text-tertiary);
		pointer-events: none;
		float: left;
		height: 0;
		padding-left: 2px;
	}

	/* Show placeholder on paragraphs inside collapsible section summary and content */
	:global(.tiptap-wrapper .tiptap [data-type="detailsSummary"] p.is-empty::before),
	:global(.tiptap-wrapper .tiptap [data-type="detailsContent"] p.is-empty::before) {
		content: attr(data-placeholder);
		color: var(--text-tertiary);
		pointer-events: none;
		float: left;
		height: 0;
		padding-left: 2px;
	}


	.link-context-overlay {
		position: fixed;
		inset: 0;
		z-index: 1500;
	}

	.link-context-menu {
		position: fixed;
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 10px;
		box-shadow: var(--shadow-lg);
		padding: 4px;
		min-width: 180px;
		z-index: 1501;
	}

	.link-context-url {
		padding: 6px 12px;
		font-size: 11px;
		color: var(--text-tertiary);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		max-width: 240px;
		border-bottom: 1px solid var(--border-light);
		margin-bottom: 4px;
	}

	.link-context-menu button {
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

	.link-context-menu button:hover {
		background: var(--bg-hover);
	}

	.link-context-menu button.danger {
		color: var(--danger);
	}

	.link-context-menu button.danger:hover {
		background: color-mix(in srgb, var(--danger) 10%, transparent);
	}

	.link-context-sep {
		height: 1px;
		background: var(--border-light);
		margin: 4px 0;
	}

	.link-modal-overlay {
		position: fixed;
		inset: 0;
		background: rgba(0, 0, 0, 0.4);
		display: flex;
		align-items: center;
		justify-content: center;
		z-index: 2000;
	}

	.link-modal {
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 12px;
		box-shadow: var(--shadow-lg);
		padding: 20px;
		width: 400px;
		max-width: 90vw;
	}

	.link-modal-header {
		display: flex;
		align-items: center;
		gap: 8px;
		margin-bottom: 16px;
		font-size: 15px;
		font-weight: 600;
		color: var(--text-primary);
	}

	.link-modal-input {
		width: 100%;
		padding: 10px 12px;
		border: 1px solid var(--border-color);
		border-radius: 8px;
		background: var(--bg-secondary);
		color: var(--text-primary);
		font-size: 14px;
		outline: none;
		box-sizing: border-box;
	}

	.link-modal-input:focus {
		border-color: var(--accent);
		box-shadow: 0 0 0 2px var(--accent-light);
	}

	.link-modal-actions {
		display: flex;
		justify-content: flex-end;
		gap: 8px;
		margin-top: 16px;
	}

	.link-modal-btn {
		padding: 7px 16px;
		border-radius: 8px;
		font-size: 13px;
		font-weight: 500;
		cursor: pointer;
		border: none;
	}

	.link-modal-btn.cancel {
		background: var(--bg-tertiary);
		color: var(--text-secondary);
	}

	.link-modal-btn.cancel:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.link-modal-btn.confirm {
		background: var(--accent);
		color: white;
	}

	.link-modal-btn.confirm:hover {
		opacity: 0.9;
	}

	.link-suggest-list {
		max-height: 240px;
		overflow-y: auto;
		margin-top: 8px;
		border: 1px solid var(--border-light);
		border-radius: 8px;
		padding: 4px;
	}

	.link-suggest-item {
		display: flex;
		align-items: center;
		gap: 8px;
		width: 100%;
		padding: 7px 10px;
		border: none;
		border-radius: 6px;
		background: none;
		color: var(--text-primary);
		font-size: 13px;
		cursor: pointer;
		text-align: left;
	}

	.link-suggest-item:hover,
	.link-suggest-item.selected {
		background: var(--accent-light);
		color: var(--accent);
	}

	.link-suggest-item svg {
		flex-shrink: 0;
		color: var(--text-tertiary);
	}

	.link-suggest-item:hover svg,
	.link-suggest-item.selected svg {
		color: var(--accent);
	}

	.link-suggest-title {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	/* Text context menu */
	.text-ctx-overlay {
		position: fixed;
		inset: 0;
		z-index: 1500;
	}

	.text-ctx-menu {
		position: fixed;
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 10px;
		box-shadow: var(--shadow-lg);
		padding: 4px;
		min-width: 200px;
		z-index: 1501;
	}

	.text-ctx-menu button {
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

	.text-ctx-menu button:hover {
		background: var(--bg-hover);
	}

	.text-ctx-shortcut {
		margin-left: auto;
		font-size: 11px;
		color: var(--text-tertiary);
		font-family: inherit;
	}

	.text-ctx-sep {
		height: 1px;
		background: var(--border-light);
		margin: 4px 0;
	}

	.text-ctx-submenu-wrap {
		position: relative;
	}

	.text-ctx-submenu-wrap > button.has-submenu {
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

	.text-ctx-submenu-wrap > button.has-submenu:hover {
		background: var(--bg-hover);
	}

	.text-ctx-submenu-wrap .submenu-arrow {
		margin-left: auto;
		color: var(--text-tertiary);
	}

	.text-ctx-submenu {
		position: absolute;
		left: 100%;
		top: -4px;
		margin-left: 2px;
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 10px;
		box-shadow: var(--shadow-lg);
		padding: 4px;
		min-width: 140px;
		z-index: 1502;
	}

	.text-ctx-submenu.flip-left {
		left: auto;
		right: 100%;
		margin-left: 0;
		margin-right: 2px;
	}

	.text-ctx-submenu button {
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

	.text-ctx-submenu button:hover {
		background: var(--bg-hover);
	}

	.text-ctx-submenu button.active {
		color: var(--accent);
		font-weight: 600;
	}

	/* Table grid size picker */
	.table-picker-dropdown {
		padding: 8px;
		min-width: auto;
		width: auto;
	}

	.table-picker-grid {
		display: grid;
		grid-template-columns: repeat(10, 18px);
		grid-template-rows: repeat(8, 18px);
		gap: 2px;
	}

	.table-picker-cell {
		width: 18px;
		height: 18px;
		border: 1px solid var(--border-color);
		border-radius: 2px;
		cursor: pointer;
		transition: background 0.05s;
		background: var(--bg-secondary);
	}

	.table-picker-cell:hover,
	.table-picker-cell.active {
		background: var(--accent-light);
		border-color: var(--accent);
	}

	.table-picker-label {
		text-align: center;
		font-size: 11px;
		color: var(--text-secondary);
		margin-top: 6px;
		font-weight: 500;
	}

	/* Table context menu */
	.table-ctx-overlay {
		position: fixed;
		inset: 0;
		z-index: 1500;
	}

	.table-ctx-menu {
		position: fixed;
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 10px;
		box-shadow: var(--shadow-lg);
		padding: 4px;
		min-width: 200px;
		max-height: calc(100vh - 16px);
		overflow-y: auto;
		z-index: 1501;
	}

	.table-ctx-menu button {
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

	.table-ctx-menu button:hover {
		background: var(--bg-hover);
	}

	.table-ctx-menu button.danger {
		color: var(--danger);
	}

	.table-ctx-menu button.danger:hover {
		background: color-mix(in srgb, var(--danger) 10%, transparent);
	}

	.table-ctx-sep {
		height: 1px;
		background: var(--border-light);
		margin: 4px 0;
	}

	.table-ctx-color-label {
		padding: 4px 12px 2px;
		font-size: 11px;
		font-weight: 600;
		color: var(--text-tertiary);
		text-transform: uppercase;
		letter-spacing: 0.03em;
	}

	.table-ctx-colors {
		display: grid;
		grid-template-columns: repeat(5, 1fr);
		gap: 4px;
		padding: 4px 10px 6px;
	}

	.table-ctx-color-swatch {
		width: 26px;
		height: 26px;
		border-radius: 5px;
		border: 1px solid var(--border-color);
		cursor: pointer;
		transition: transform 0.1s;
		padding: 0;
	}

	.table-ctx-color-swatch:hover {
		transform: scale(1.15);
		border-color: var(--accent);
	}

	/* PDF embeds */
	:global(.tiptap .math-block) {
		margin: 16px 0;
		padding: 12px;
		text-align: center;
		overflow-x: auto;
		cursor: default;
	}
	:global(.tiptap .math-inline) {
		cursor: default;
	}
	:global(.tiptap .mermaid-render) {
		position: relative;
		margin: 4px 0 16px;
		padding: 12px;
		background: var(--bg-secondary);
		border: 1px solid var(--border);
		border-radius: 8px;
		text-align: center;
		overflow-x: auto;
		cursor: default;
		min-height: 32px;
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 8px;
	}
	:global(.tiptap .mermaid-render-toolbar) {
		position: absolute;
		top: 6px;
		right: 6px;
		display: flex;
		gap: 4px;
		opacity: 0;
		transition: opacity 0.15s ease;
	}
	:global(.tiptap .mermaid-render:hover .mermaid-render-toolbar) {
		opacity: 1;
	}
	:global(.tiptap .mermaid-render-action) {
		appearance: none;
		background: var(--bg-primary);
		color: var(--text-secondary);
		border: 1px solid var(--border);
		border-radius: 5px;
		padding: 3px 9px;
		font-size: 11px;
		cursor: pointer;
		font-family: inherit;
		line-height: 1.3;
	}
	:global(.tiptap .mermaid-render-action:hover) {
		color: var(--text-primary);
		border-color: var(--accent);
	}
	:global(.tiptap .mermaid-render-toast) {
		position: absolute;
		bottom: 8px;
		right: 8px;
		background: var(--text-primary);
		color: var(--bg-primary);
		padding: 4px 10px;
		border-radius: 5px;
		font-size: 12px;
		max-width: 80%;
		pointer-events: none;
		animation: mermaid-toast-fade 1.5s ease;
	}
	@keyframes mermaid-toast-fade {
		0% { opacity: 0; transform: translateY(4px); }
		15% { opacity: 1; transform: translateY(0); }
		70% { opacity: 1; }
		100% { opacity: 0; }
	}
	:global(.tiptap .mermaid-render svg) {
		max-width: 100%;
		height: auto;
	}
	:global(.tiptap .mermaid-render-loading::before) {
		content: 'Rendering diagram…';
		color: var(--text-secondary);
		font-size: 13px;
	}
	:global(.tiptap .mermaid-render-error) {
		color: var(--text-secondary);
		font-size: 13px;
	}
	:global(.tiptap .mermaid-render-btn) {
		appearance: none;
		background: var(--bg-tertiary, var(--bg-primary));
		color: var(--text-primary);
		border: 1px solid var(--border);
		border-radius: 6px;
		padding: 6px 14px;
		font-size: 13px;
		cursor: pointer;
		font-family: inherit;
	}
	:global(.tiptap .mermaid-render-btn:hover) {
		background: var(--accent);
		color: var(--accent-fg, white);
		border-color: var(--accent);
	}
	:global(.tiptap .mermaid-render-btn-small) {
		padding: 3px 10px;
		font-size: 12px;
	}
	:global(.tiptap .pdf-embed) {
		margin: 12px 0;
		border: 1px solid var(--border);
		border-radius: 8px;
		overflow: hidden;
		background: var(--bg-secondary);
	}
	:global(.tiptap .pdf-embed iframe) {
		display: block;
		border: none;
		width: 100%;
	}
	:global(.tiptap .pdf-embed .pdf-label) {
		padding: 6px 12px;
		font-size: 12px;
		color: var(--text-secondary);
		border-top: 1px solid var(--border);
		margin: 0;
	}
	:global(.tiptap .page-break) {
		position: relative;
		margin: 12px 0;
		height: 20px;
		border: none;
		pointer-events: none;
		user-select: none;
	}
	:global(.tiptap .page-break::before) {
		content: '';
		position: absolute;
		top: 50%;
		left: 0;
		right: 0;
		height: 0;
		border-top: 2px dashed var(--text-tertiary, #aaa);
		opacity: 0.5;
	}
	:global(.tiptap .page-break::after) {
		content: 'Page Break';
		position: absolute;
		top: 50%;
		left: 50%;
		transform: translate(-50%, -50%);
		background: var(--bg-primary, #fff);
		padding: 0 8px;
		font-size: 11px;
		color: var(--text-tertiary, #aaa);
		white-space: nowrap;
	}
	:global(.tiptap .pdf-embed-mobile) {
		margin: 8px 0;
	}
	:global(.tiptap .pdf-link-mobile) {
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 14px 16px;
		background: var(--bg-secondary);
		border: 1px solid var(--border-color);
		border-radius: 10px;
		color: var(--text-primary);
		font-size: 15px;
		font-weight: 500;
		text-decoration: none;
		cursor: pointer;
	}
	:global(.tiptap .pdf-link-mobile:active) {
		background: var(--bg-hover);
	}
	:global(.tiptap .pdf-icon-mobile) {
		font-size: 20px;
		line-height: 1;
	}

	/* Slash commands menu */
	.slash-menu-overlay {
		position: fixed;
		inset: 0;
		z-index: 1500;
	}

	.task-due-input {
		position: fixed;
		z-index: 1501;
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 6px;
		padding: 6px 8px;
		color: var(--text-primary);
		font: inherit;
		box-shadow: var(--shadow-lg);
	}

	.slash-menu {
		position: fixed;
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 10px;
		box-shadow: var(--shadow-lg);
		padding: 4px;
		min-width: 220px;
		max-height: 300px;
		overflow-y: auto;
		z-index: 1501;
	}

	.slash-menu::-webkit-scrollbar {
		width: 4px;
	}

	.slash-menu::-webkit-scrollbar-thumb {
		background: var(--text-tertiary);
		border-radius: 2px;
	}

	.slash-menu-item {
		display: flex;
		align-items: center;
		gap: 10px;
		width: 100%;
		padding: 8px 12px;
		border: none;
		background: none;
		color: var(--text-primary);
		font-size: 13px;
		cursor: pointer;
		border-radius: 6px;
		text-align: left;
	}

	.slash-menu-item:hover,
	.slash-menu-item.selected {
		background: var(--bg-hover);
	}

	.slash-menu-icon {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 28px;
		height: 28px;
		border-radius: 6px;
		background: color-mix(in srgb, var(--accent) 12%, transparent);
		color: var(--accent);
		flex-shrink: 0;
	}

	.slash-menu-label {
		flex: 1;
	}

	.slash-menu-empty {
		padding: 12px 16px;
		color: var(--text-tertiary);
		font-size: 13px;
		text-align: center;
	}

	.slash-table-picker {
		padding: 8px;
	}

	.slash-table-picker-grid {
		display: grid;
		grid-template-columns: repeat(10, 18px);
		grid-template-rows: repeat(8, 18px);
		gap: 2px;
	}

	.slash-table-picker-label {
		text-align: center;
		font-size: 11px;
		color: var(--text-secondary);
		margin-top: 6px;
		font-weight: 500;
	}

	/* Color swatch preview shown before a color literal (VSCode-style decorator) */
	:global(.tiptap-wrapper .tiptap .color-swatch) {
		display: inline-block;
		width: 0.8em;
		height: 0.8em;
		border-radius: 3px;
		margin-right: 0.3em;
		vertical-align: baseline;
		border: 1px solid var(--border-color);
		box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.15);
		user-select: none;
	}

	/* /color slash sub-picker */
	.slash-color-picker {
		padding: 8px;
		width: 210px;
	}
	.slash-color-swatches {
		display: grid;
		grid-template-columns: repeat(6, 1fr);
		gap: 6px;
		margin-bottom: 8px;
	}
	.slash-color-swatch {
		width: 100%;
		aspect-ratio: 1;
		border-radius: 5px;
		border: 1px solid var(--border-color);
		box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.12);
		cursor: pointer;
		padding: 0;
	}
	.slash-color-swatch:hover {
		transform: scale(1.1);
	}
	.slash-color-row {
		display: flex;
		align-items: center;
		gap: 6px;
	}
	.slash-color-native {
		width: 28px;
		height: 28px;
		padding: 0;
		border: 1px solid var(--border-color);
		border-radius: 6px;
		background: none;
		cursor: pointer;
		flex-shrink: 0;
	}
	.slash-color-input {
		flex: 1;
		min-width: 0;
		padding: 6px 8px;
		border: 1px solid var(--border-color);
		border-radius: 6px;
		background: var(--bg-secondary);
		color: var(--text-primary);
		font-size: 12px;
		font-family: monospace;
		outline: none;
	}
	.slash-color-insert {
		padding: 6px 10px;
		border: none;
		border-radius: 6px;
		background: var(--accent);
		color: #fff;
		font-size: 12px;
		font-weight: 500;
		cursor: pointer;
		flex-shrink: 0;
	}

	/* AI Menu */
	.ai-menu-overlay {
		position: fixed;
		inset: 0;
		z-index: 1600;
	}

	.ai-menu {
		position: fixed;
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 10px;
		box-shadow: var(--shadow-lg);
		padding: 4px;
		min-width: 320px;
		max-width: 480px;
		z-index: 1601;
		max-height: 80vh;
		overflow-y: auto;
	}

	.ai-menu-label {
		padding: 6px 12px 4px;
		font-size: 11px;
		font-weight: 600;
		color: var(--text-tertiary);
		text-transform: uppercase;
		letter-spacing: 0.03em;
	}

	.ai-menu-item {
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

	.ai-menu-item:hover {
		background: var(--bg-hover);
	}

	.ai-menu-sep {
		height: 1px;
		background: var(--border-light);
		margin: 4px 0;
	}

	.ai-menu-arrow {
		margin-left: auto;
		color: var(--text-tertiary);
		display: flex;
		align-items: center;
	}

	.ai-result-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 8px 12px 4px;
	}

	.ai-result-title {
		font-size: 12px;
		font-weight: 600;
		color: var(--text-secondary);
		display: flex;
		align-items: center;
		gap: 6px;
	}

	.ai-spinner {
		animation: spin 0.8s linear infinite;
	}

	@keyframes spin {
		from { transform: rotate(0deg); }
		to { transform: rotate(360deg); }
	}

	.ai-result-close {
		background: none;
		border: none;
		color: var(--text-tertiary);
		cursor: pointer;
		padding: 2px;
		border-radius: 4px;
		display: flex;
		align-items: center;
	}

	.ai-result-close:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.ai-result-body {
		padding: 8px 12px;
		font-size: 13px;
		color: var(--text-primary);
		line-height: 1.6;
		max-height: 50vh;
		overflow-y: auto;
		white-space: pre-wrap;
		word-break: break-word;
	}

	.ai-error {
		padding: 8px 12px;
		font-size: 12px;
		color: var(--danger);
		line-height: 1.5;
	}

	.ai-result-actions {
		display: flex;
		gap: 6px;
		padding: 6px 10px 8px;
		border-top: 1px solid var(--border-light);
	}

	.ai-action-btn {
		flex: 1;
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 4px;
		padding: 6px 10px;
		border: none;
		border-radius: 6px;
		font-size: 12px;
		font-weight: 500;
		cursor: pointer;
		transition: all 0.15s;
	}

	.ai-action-btn.apply {
		background: var(--accent);
		color: white;
	}

	.ai-action-btn.apply:hover {
		opacity: 0.9;
	}

	.ai-action-btn.discard {
		background: var(--bg-secondary);
		border: 1px solid var(--border-color);
		color: var(--text-secondary);
	}

	.ai-action-btn.discard:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.ai-custom-header {
		display: flex;
		align-items: center;
		gap: 6px;
		padding: 8px 12px 4px;
		font-size: 12px;
		font-weight: 600;
		color: var(--text-secondary);
	}

	.ai-back-btn {
		background: none;
		border: none;
		color: var(--text-tertiary);
		cursor: pointer;
		padding: 2px;
		border-radius: 4px;
		display: flex;
		align-items: center;
	}

	.ai-back-btn:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.ai-custom-body {
		padding: 6px 10px 8px;
	}

	.ai-custom-input {
		width: 100%;
		min-height: 60px;
		padding: 8px 10px;
		border: 1px solid var(--border-color);
		border-radius: 6px;
		background: var(--bg-secondary);
		color: var(--text-primary);
		font-size: 13px;
		font-family: inherit;
		resize: vertical;
		outline: none;
		box-sizing: border-box;
	}

	.ai-custom-input:focus {
		border-color: var(--accent);
	}

	.ai-custom-input::placeholder {
		color: var(--text-tertiary);
	}

	.ai-custom-submit {
		margin-top: 6px;
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 4px;
		width: 100%;
		padding: 7px 12px;
		border: none;
		border-radius: 6px;
		background: var(--accent);
		color: white;
		font-size: 12px;
		font-weight: 500;
		cursor: pointer;
	}

	.ai-custom-submit:hover:not(:disabled) {
		opacity: 0.9;
	}

	.ai-custom-submit:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}

	/* Wiki-link styles */
	:global(.tiptap-wrapper .tiptap .wiki-link) {
		color: var(--accent);
		text-decoration: underline dotted;
		text-underline-offset: 3px;
		cursor: pointer;
		border-radius: 2px;
		padding: 0 1px;
	}

	:global(.tiptap-wrapper .tiptap .wiki-link:hover) {
		background: var(--accent-light);
		text-decoration: underline solid;
	}

	:global(.tiptap-wrapper .tiptap .wiki-link[data-path=""]) {
		color: var(--text-tertiary);
		text-decoration: underline dashed;
	}

	.wiki-link-overlay {
		position: fixed;
		inset: 0;
		z-index: 1600;
	}

	.wiki-link-menu {
		position: fixed;
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 10px;
		box-shadow: var(--shadow-lg);
		width: 280px;
		max-height: 360px;
		overflow-y: auto;
		padding: 4px;
		z-index: 1601;
	}

	.wiki-link-empty {
		padding: 12px 14px;
		font-size: 12px;
		color: var(--text-tertiary);
		text-align: center;
	}

	.wiki-link-item {
		display: flex;
		align-items: center;
		gap: 8px;
		width: 100%;
		padding: 7px 10px;
		border: none;
		border-radius: 6px;
		background: none;
		color: var(--text-primary);
		font-size: 13px;
		cursor: pointer;
		text-align: left;
	}

	.wiki-link-item:hover,
	.wiki-link-item.selected {
		background: var(--accent-light);
		color: var(--accent);
	}

	.wiki-link-item svg {
		flex-shrink: 0;
		color: var(--text-tertiary);
	}

	.wiki-link-item:hover svg,
	.wiki-link-item.selected svg {
		color: var(--accent);
	}

	.wiki-link-title-col {
		display: flex;
		flex-direction: column;
		overflow: hidden;
		min-width: 0;
	}

	.wiki-link-title {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.wiki-link-folder {
		font-size: 11px;
		color: var(--text-tertiary);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.wiki-link-disambig-header {
		padding: 8px 10px 4px;
		font-size: 11px;
		color: var(--text-tertiary);
		font-weight: 500;
	}

	/* ═══ MOBILE (class-based, not media-query, for Android high-DPI) ═══ */
	.editor-container.mobile {
		height: 100%;
		min-height: 0;
		overflow: hidden;
	}

	.editor-container.mobile .editor-toolbar {
		padding: 8px 16px 6px 16px;
		flex-shrink: 0;
		flex-direction: column;
		flex-wrap: nowrap;
		align-items: stretch;
		gap: 2px;
	}

	.editor-container.mobile .editor-title {
		min-width: 0;
	}

	.editor-container.mobile .toolbar-actions {
		gap: 4px;
		margin-left: 0;
	}

	.editor-container.mobile .toolbar-actions .icon-btn {
		min-width: 32px;
		min-height: 32px;
		display: flex;
		align-items: center;
		justify-content: center;
	}

	.editor-container.mobile .toolbar-actions .save-indicator,
	.editor-container.mobile .toolbar-actions .readonly-indicator {
		font-size: 12px;
	}

	.editor-container.mobile .editor-title input {
		font-size: 20px;
		padding: 4px 0;
	}

	.editor-container.mobile .editor-body-wrapper {
		flex: 1;
		min-height: 0;
		overflow: hidden;
	}

	.editor-container.mobile .editor-body-row {
		min-height: 0;
	}

	.editor-container.mobile .editor-body {
		min-height: 0;
		overflow-y: auto;
		-webkit-overflow-scrolling: touch;
		padding: 0;
	}

	.editor-container.mobile .tiptap-wrapper {
		min-height: 0;
	}

	.editor-container.mobile .editor-formatting-bar {
		position: fixed;
		bottom: 0;
		left: 0;
		right: 0;
		z-index: 50;
		padding: 4px 8px;
		padding-bottom: calc(4px + env(safe-area-inset-bottom));
		overflow: visible;
		flex-wrap: nowrap;
		gap: 1px;
		background: var(--bg-secondary);
		border-top: 1px solid var(--border-color);
	}

	.editor-container.mobile .fmt-main-controls::-webkit-scrollbar {
		display: none;
	}

	.editor-container.mobile .fmt-main-controls {
		display: flex;
		flex: 1;
		min-width: 0;
		align-items: center;
		flex-wrap: nowrap;
		gap: 1px;
		overflow-x: auto;
		scrollbar-width: none;
		-webkit-overflow-scrolling: touch;
	}


	.editor-container.mobile .table-picker-grid {
		grid-template-columns: repeat(10, 44px);
		grid-template-rows: repeat(8, 44px);
	}

	.editor-container.mobile .table-picker-dropdown .table-picker-cell {
		width: 44px;
		height: 44px;
		padding: 0;
	}

	.editor-container.mobile .fmt-btn {
		min-width: 38px;
		height: 38px;
		flex-shrink: 0;
		padding: 6px;
	}

	.editor-container.mobile .fmt-sep {
		height: 20px;
		margin: 0 2px;
	}

	.editor-container.mobile .fmt-dropdown {
		min-width: 180px;
	}

	.editor-container.mobile .fmt-dropdown button {
		padding: 12px 16px;
		font-size: 15px;
		min-height: 44px;
	}

	.editor-container.mobile .insert-dropdown {
		width: max-content;
		min-width: 200px;
	}

	.editor-container.mobile .shortcuts-hint {
		display: none;
	}

	.editor-container.mobile .empty-editor p {
		font-size: 16px;
	}

	.editor-container.mobile :global(.editor-content) {
		padding: 8px 16px 220px !important;
		/* Respect the user's font-size setting; 16px is the default. (issue #100)
		   Auto-zoom isn't a concern: the viewport sets user-scalable=no. */
		font-size: var(--editor-font-size, 16px) !important;
	}


	.editor-container.mobile .editor-body-row {
		position: relative;
	}

	.editor-container.mobile .editor-body-row:has(.outline-panel) > .editor-body {
		display: none;
	}

	.editor-container.mobile .outline-panel {
		position: static;
		flex: 1;
		min-height: 0;
		width: 100% !important;
		max-width: 100%;
		border-left: none;
	}

	.editor-container.mobile .outline-item { min-height: 44px; padding-top: 10px; padding-bottom: 10px; }
	.editor-container.mobile .outline-close { min-width: 44px; min-height: 44px; justify-content: center; }
	.editor-container.mobile :global(.resize-handle) { display: none; }

	.editor-container.mobile .editor-body-row.reviewing-history { flex-direction: column-reverse; }
	.editor-container.mobile .history-panel { width: 100%; flex: none; border-left: none; border-bottom: 1px solid var(--border-light); }
	.editor-container.mobile .history-list { display: none; }
	.editor-container.mobile .history-picker { display: flex; flex-direction: column; gap: 5px; padding: 0 12px 10px; }
	.editor-container.mobile .history-picker select { min-height: 44px; }

	.editor-container.mobile .history-item {
		padding: 12px 12px;
		min-height: 48px;
	}

	.editor-container.mobile .history-create-btn,
	.editor-container.mobile .history-close,
	.editor-container.mobile .history-retry-btn {
		min-width: 44px;
		min-height: 44px;
	}


	.editor-container.mobile .note-search-bar {
		padding: 8px 12px;
	}

	.editor-container.mobile .note-search-bar input {
		padding: 8px 10px;
		font-size: 15px;
	}

	/* ═══ AI Menu - Mobile Bottom Sheet ═══ */
	.ai-menu-overlay.mobile {
		background: rgba(0, 0, 0, 0.35);
		display: flex;
		align-items: flex-end;
		justify-content: center;
	}

	.ai-menu.mobile {
		position: relative;
		left: auto !important;
		top: auto !important;
		width: 100%;
		max-width: 100%;
		min-width: 0;
		border-radius: 16px 16px 0 0;
		border-bottom: none;
		max-height: 70vh;
		padding: 8px 4px calc(env(safe-area-inset-bottom, 0px) + 8px);
	}

	.ai-menu.mobile .ai-menu-label {
		padding: 10px 16px 6px;
		font-size: 12px;
	}

	.ai-menu.mobile .ai-menu-item {
		padding: 12px 16px;
		font-size: 15px;
		min-height: 44px;
		border-radius: 8px;
	}

	.ai-menu.mobile .ai-menu-sep {
		margin: 4px 8px;
	}

	.ai-menu.mobile .ai-result-header {
		padding: 12px 16px 8px;
	}

	.ai-menu.mobile .ai-result-body {
		padding: 8px 16px;
		font-size: 15px;
		max-height: 40vh;
	}

	.ai-menu.mobile .ai-result-actions {
		padding: 8px 16px 4px;
		gap: 10px;
	}

	.ai-menu.mobile .ai-result-actions button {
		padding: 10px 16px;
		font-size: 14px;
		min-height: 44px;
	}

	.ai-menu.mobile .ai-custom-body {
		padding: 8px 12px;
	}

	.ai-menu.mobile .ai-custom-input {
		font-size: 15px;
		min-height: 80px;
	}

	.ai-menu.mobile .ai-custom-submit {
		padding: 10px 16px;
		font-size: 14px;
		min-height: 44px;
	}

	.ai-menu.mobile .ai-custom-header {
		padding: 10px 12px;
		font-size: 15px;
	}

	.ai-menu.mobile .ai-back-btn {
		min-width: 44px;
		min-height: 44px;
	}

	.ai-menu.mobile .ai-error {
		padding: 12px 16px;
		font-size: 14px;
	}

	/* ═══ Info Panel ═══ */
	.info-panel {
		width: 260px;
		border-left: 1px solid var(--border-light);
		background: var(--bg-secondary);
		display: flex;
		flex-direction: column;
		overflow-y: auto;
		flex-shrink: 0;
	}

	.info-panel-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 12px 14px;
		border-bottom: 1px solid var(--border-light);
		flex-shrink: 0;
	}

	.info-panel-title {
		font-size: 12px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--text-tertiary);
	}

	.info-close-btn {
		background: none;
		border: none;
		color: var(--text-tertiary);
		cursor: pointer;
		padding: 2px;
		border-radius: 4px;
		display: flex;
		align-items: center;
	}

	.info-close-btn:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.info-section {
		padding: 12px 14px;
		border-bottom: 1px solid var(--border-light);
	}

	.info-section:last-child {
		border-bottom: none;
	}

	.info-details {
		flex: 1;
	}

	.info-actions {
		padding: 12px 14px;
	}

	.info-trash-btn {
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 7px;
		width: 100%;
		min-height: 36px;
		border: 1px solid color-mix(in srgb, var(--danger) 30%, var(--border-color));
		border-radius: 6px;
		background: transparent;
		color: var(--danger);
		font: inherit;
		font-size: 12px;
		font-weight: 600;
		cursor: pointer;
	}

	.info-trash-btn:hover {
		background: color-mix(in srgb, var(--danger) 10%, transparent);
	}

	.info-trash-btn:focus-visible {
		outline: 2px solid var(--danger);
		outline-offset: 2px;
	}

	.info-trash-btn:disabled {
		opacity: 0.45;
		cursor: default;
	}

	.info-section-label {
		font-size: 11px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--text-tertiary);
		margin-bottom: 8px;
	}

	.info-frontmatter { flex-shrink: 0; }
	.info-frontmatter > summary { cursor: pointer; margin-bottom: 0; padding: 2px 0; border-radius: 4px; }
	.info-frontmatter[open] > summary { margin-bottom: 8px; }
	.info-frontmatter > summary:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
	.info-frontmatter-caption { margin: 0 0 8px; font-size: 11px; line-height: 1.5; color: var(--text-secondary); }
	.info-frontmatter pre { margin: 0; font-family: var(--font-mono, monospace); font-size: 11px; line-height: 1.6; color: var(--text-primary); white-space: pre-wrap; overflow-wrap: anywhere; }
	@media (max-width: 600px) {
		.info-frontmatter > summary { min-height: 44px; padding: 12px 0; }
	}
	.editor-container.mobile .info-frontmatter > summary { min-height: 44px; padding: 12px 0; }

	.info-row {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		gap: 8px;
		padding: 3px 0;
	}

	.info-row-tags {
		align-items: flex-start;
	}

	.info-key {
		font-size: 12px;
		color: var(--text-tertiary);
		flex-shrink: 0;
	}

	.info-value {
		font-size: 12px;
		color: var(--text-primary);
		text-align: right;
		font-variant-numeric: tabular-nums;
	}

	.info-value-path {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		max-width: 140px;
	}

	.info-note-path {
		min-width: 0;
		margin-top: 8px;
		padding-top: 8px;
		border-top: 1px solid var(--border-light);
	}

	.info-note-path-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
	}

	.info-copy-path-btn {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		margin: -3px -4px -3px 0;
		padding: 3px 5px;
		border: none;
		border-radius: 4px;
		background: transparent;
		color: var(--accent);
		font-size: 11px;
		font-weight: 600;
		cursor: pointer;
	}

	.info-copy-path-btn:hover {
		background: var(--accent-light);
	}

	.info-copy-path-btn:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: 1px;
	}

	.info-copy-path-btn.copied {
		color: var(--success);
	}

	.info-copy-path-btn.error {
		color: var(--danger);
	}

	.info-note-path-value {
		display: block;
		margin-top: 4px;
		overflow: hidden;
		color: var(--text-secondary);
		font-family: "JetBrains Mono", ui-monospace, SFMono-Regular, Consolas, monospace;
		font-size: 10px;
		line-height: 1.4;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.info-tags {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
		justify-content: flex-end;
	}

	.editor-container.mobile .editor-body-row:has(.info-panel) > .editor-body {
		display: none;
	}

	.editor-container.mobile .info-panel {
		width: 100%;
		border-left: none;
		border-top: 1px solid var(--border-light);
	}

	.editor-container.mobile .info-trash-btn {
		min-height: 44px;
		font-size: 14px;
	}
</style>
