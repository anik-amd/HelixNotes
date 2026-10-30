import { Extension } from '@tiptap/core';
import { Plugin, PluginKey, TextSelection, type EditorState, type Transaction } from '@tiptap/pm/state';
import { getSelectionPair } from '../selectionPairs.ts';

/**
 * Wrap the selected text in the typed pair, or null to let the input through.
 *
 * ProseMirror reports every text change of the DOM here, not only keystrokes. A compose key
 * or dead key shows what has been typed so far as a preedit (`·`, then `'` for Compose ' e)
 * and replaces it at each step, which arrives as a quote replacing a non-empty range. Only
 * the user's own selection is wrapped: the range must be the selection, outside composition.
 */
export function wrapSelection(state: EditorState, from: number, to: number, text: string, composing = false): Transaction | null {
	const close = getSelectionPair(text);
	if (!close || from === to || composing) return null;
	const { selection } = state;
	if (selection.empty || selection.from !== from || selection.to !== to) return null;
	const tr = state.tr.insertText(close, to).insertText(text, from);
	tr.setSelection(TextSelection.create(tr.doc, from + text.length, to + text.length));
	return tr;
}

// TipTap/ProseMirror behavior for wrapping a non-empty rich-editor selection
// without flattening selected inline marks or content to plain text.
export const WrapSelectedText = Extension.create({
	name: 'wrapSelectedText',

	addProseMirrorPlugins() {
		return [
			new Plugin({
				key: new PluginKey('wrapSelectedText'),
				props: {
					// Intercept only supported single-character text input over the selection.
					// Everything else falls back to ProseMirror's normal input handling.
					handleTextInput(view, from, to, text) {
						const tr = wrapSelection(view.state, from, to, text, view.composing);
						if (!tr) return false;
						view.dispatch(tr);
						return true;
					},
				},
			}),
		];
	},
});
