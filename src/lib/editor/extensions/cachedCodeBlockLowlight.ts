import CodeBlock, { type CodeBlockOptions } from '@tiptap/extension-code-block';
import { toggleSelectedCode } from './toggleSelectedCode.ts';
import type { Node as ProseMirrorNode } from '@tiptap/pm/model';
import { Plugin, PluginKey } from '@tiptap/pm/state';
import { Decoration, DecorationSet } from '@tiptap/pm/view';

interface HighlightNode {
	type: string;
	value?: string;
	properties?: { className?: unknown };
	children?: HighlightNode[];
}

export interface CodeHighlighter {
	registered(language: string): boolean;
	highlight(language: string, text: string): { children: HighlightNode[] };
	highlightAuto(text: string): { children: HighlightNode[] };
}
type HighlightSpan = { from: number; to: number; className: string };
type HighlightState = { blocks: Map<number, ProseMirrorNode>; decorations: DecorationSet };

export function createCodeBlockHighlightPlugin({
	name = 'codeBlock',
	lowlight,
	defaultLanguage = null,
}: {
	name?: string;
	lowlight: CodeHighlighter;
	defaultLanguage?: string | null;
}) {
	// Nodes are immutable. Relative spans survive moves and undo without retaining
	// deleted documents, and never leak between editor/highlighter instances.
	const cache = new WeakMap<ProseMirrorNode, HighlightSpan[]>();

	function highlight(node: ProseMirrorNode): HighlightSpan[] {
		const cached = cache.get(node);
		if (cached) return cached;
		const language = node.attrs.language || defaultLanguage;
		// A named language without a grammar (mermaid, a typo) stays plain: guessing would run
		// every registered grammar over the block on each edit.
		const tree = !language ? lowlight.highlightAuto(node.textContent)
			: lowlight.registered(language) ? lowlight.highlight(language, node.textContent)
			: { children: [] };
		const spans: HighlightSpan[] = [];
		let offset = 0;
		function visit(nodes: HighlightNode[], inheritedClass: string) {
			for (const child of nodes) {
				if (child.type === 'element' && child.children) {
					const classes = child.properties?.className;
					const ownClass = Array.isArray(classes) ? classes.join(' ') : typeof classes === 'string' ? classes : '';
					visit(child.children, [inheritedClass, ownClass].filter(Boolean).join(' '));
				} else if (child.type === 'text' && child.value !== undefined) {
					const end = offset + child.value.length;
					if (inheritedClass && end > offset) spans.push({ from: offset, to: end, className: inheritedClass });
					offset = end;
				}
			}
		}
		visit(tree.children, '');
		cache.set(node, spans);
		return spans;
	}

	function collect(doc: ProseMirrorNode) {
		const blocks = new Map<number, ProseMirrorNode>();
		doc.descendants((node, pos) => {
			if (node.type.name !== name) return !node.isTextblock;
			blocks.set(pos, node);
			return false;
		});
		return blocks;
	}

	function decorate(blocks: Map<number, ProseMirrorNode>, retained?: Set<number>) {
		const decorations: Decoration[] = [];
		for (const [pos, node] of blocks) {
			if (retained?.has(pos)) continue;
			for (const span of highlight(node)) {
				decorations.push(Decoration.inline(pos + 1 + span.from, pos + 1 + span.to, { class: span.className }));
			}
		}
		return decorations;
	}

	const plugin: Plugin<HighlightState> = new Plugin({
		key: new PluginKey('cachedCodeBlockLowlight'),
		state: {
			init: (_, { doc }) => {
				const blocks = collect(doc);
				return { blocks, decorations: DecorationSet.create(doc, decorate(blocks)) };
			},
			apply(transaction, previous) {
				// Cursor motion must not even walk the document.
				if (!transaction.docChanged) return previous;
				const blocks = collect(transaction.doc);
				const retained = new Set<number>();
				const removed: Decoration[] = [];
				for (const [pos, node] of previous.blocks) {
					const from = transaction.mapping.mapResult(pos, 1);
					const to = transaction.mapping.mapResult(pos + node.nodeSize, -1);
					if (!from.deleted && !to.deleted && to.pos - from.pos === node.nodeSize && blocks.get(from.pos) === node) {
						retained.add(from.pos);
					} else {
						removed.push(...previous.decorations.find(pos + 1, pos + node.nodeSize - 1));
					}
				}
				// Drop changed/removed blocks before mapping, including code converted to
				// a paragraph where a markup-only step leaves its text positions intact.
				const mapped = previous.decorations.remove(removed).map(transaction.mapping, transaction.doc);
				return { blocks, decorations: mapped.add(transaction.doc, decorate(blocks, retained)) };
			},
		},
		props: {
			decorations(state) {
				return plugin.getState(state)?.decorations;
			},
		},
	});
	return plugin;
}

export const CachedCodeBlockLowlight = CodeBlock.extend<CodeBlockOptions & { lowlight: CodeHighlighter | null }>({
	addOptions() {
		return { ...this.parent!(), lowlight: null };
	},
	addCommands() {
		return { ...this.parent?.(), toggleCodeBlock: (attributes) => toggleSelectedCode(attributes) };
	},
	addAttributes() {
		// A block written without a language keeps none, so a bare fence is not saved as the default
		// language. Highlighting still falls back to the default language for such blocks.
		const attributes: Record<string, object> = this.parent?.() ?? {};
		return { ...attributes, language: { ...attributes.language, default: null } };
	},
	addProseMirrorPlugins() {
		if (!this.options.lowlight) throw new Error('Code block highlighting requires a lowlight instance');
		return [
			...(this.parent?.() ?? []),
			createCodeBlockHighlightPlugin({ name: this.name, lowlight: this.options.lowlight, defaultLanguage: this.options.defaultLanguage }),
		];
	},
});
