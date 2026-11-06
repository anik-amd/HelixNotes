import type { Command } from '@tiptap/core';
import { TextSelection } from '@tiptap/pm/state';

/** Convert a top-level text selection as a unit; leave complex structures to Tiptap. */
export function toggleSelectedCode(attributes: Record<string, unknown> = {}): Command {
	return ({ state, tr, dispatch, commands }) => {
		const { selection, schema } = state;
		if (selection.empty) return commands.toggleNode('codeBlock', 'paragraph', attributes);

		const blocks: { node: typeof state.doc; pos: number }[] = [];
		state.doc.forEach((node, pos) => {
			if (pos < selection.to && pos + node.nodeSize > selection.from) blocks.push({ node, pos });
		});
		const supported = blocks.length > 0 && blocks.every(({ node }) => {
			if (node.type.name !== 'paragraph' && node.type.name !== 'codeBlock') return false;
			let valid = true;
			node.forEach((child) => { if (!child.isText && child.type.name !== 'hardBreak') valid = false; });
			return valid;
		});
		// Keep the existing single-paragraph behaviour and do not flatten lists, images, etc.
		if (!supported || (blocks.length === 1 && blocks[0].node.type.name !== 'codeBlock')) {
			return commands.toggleNode('codeBlock', 'paragraph', attributes);
		}

		const allCode = blocks.every(({ node }) => node.type.name === 'codeBlock');
		const text = blocks.map(({ node }) => node.textBetween(0, node.content.size, '\n', '\n')).join('\n');
		const replacement = allCode
			? text.split('\n').map((line) => schema.nodes.paragraph.create(null, line ? schema.text(line) : null))
			: [schema.nodes.codeBlock.create(attributes, text ? schema.text(text) : null)];
		const from = blocks[0].pos;
		const last = blocks[blocks.length - 1];
		const to = last.pos + last.node.nodeSize;
		if (dispatch) {
			tr.replaceWith(from, to, replacement);
			const end = from + replacement.reduce((size, node) => size + node.nodeSize, 0);
			tr.setSelection(TextSelection.create(tr.doc, from + 1, end - 1));
		}
		return true;
	};
}
