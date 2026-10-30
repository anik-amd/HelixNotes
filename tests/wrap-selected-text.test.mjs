import assert from 'node:assert/strict';
import test from 'node:test';
import { Schema } from '@tiptap/pm/model';
import { EditorState, TextSelection } from '@tiptap/pm/state';
import { wrapSelection } from '../src/lib/editor/extensions/wrapSelectedText.ts';

const schema = new Schema({
  nodes: {
    doc: { content: 'block+' },
    paragraph: { group: 'block', content: 'inline*' },
    text: { group: 'inline' },
  },
  marks: { bold: {} },
});

// A document holding one paragraph with `text`; positions inside it start at 1.
function stateWith(text, anchor, head = anchor) {
  const doc = schema.node('doc', null, [schema.node('paragraph', null, schema.text(text))]);
  return EditorState.create({ doc, selection: TextSelection.create(doc, anchor, head) });
}

test('typing a pair character over the selection wraps it and keeps the text selected', () => {
  const state = stateWith('say hello now', 5, 10);
  const tr = wrapSelection(state, 5, 10, '(');
  assert.ok(tr);
  assert.equal(tr.doc.textContent, 'say (hello) now');
  assert.equal(tr.selection.from, 6);
  assert.equal(tr.selection.to, 11);
  assert.equal(tr.doc.textBetween(tr.selection.from, tr.selection.to), 'hello');
});

test('a compose or dead-key preedit being replaced is not a selection and is left alone', () => {
  // Compose ' e: GTK shows "·", then replaces it with "'", then commits "é". ProseMirror reports
  // the replacement as a quote typed over the preedit range, with the cursor collapsed after it.
  const state = stateWith('a·', 3);
  assert.equal(wrapSelection(state, 2, 3, "'", true), null);
  assert.equal(wrapSelection(state, 2, 3, "'", false), null);
});

test('only a range equal to the selection is wrapped, never a collapsed cursor', () => {
  const partial = stateWith('say hello now', 5, 10);
  assert.equal(wrapSelection(partial, 5, 8, '"'), null);
  assert.equal(wrapSelection(partial, 5, 5, '"'), null);
  const cursor = stateWith('say hello now', 5);
  assert.equal(wrapSelection(cursor, 5, 10, '"'), null);
});

test('characters without a closing pair pass through', () => {
  const state = stateWith('say hello now', 5, 10);
  assert.equal(wrapSelection(state, 5, 10, 'x'), null);
  assert.equal(wrapSelection(state, 5, 10, 'é'), null);
});
