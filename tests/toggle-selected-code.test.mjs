import test from 'node:test';
import assert from 'node:assert/strict';
import { Editor } from '@tiptap/core';
import StarterKit from '@tiptap/starter-kit';
import { createLowlight } from 'lowlight';
import { CachedCodeBlockLowlight } from '../src/lib/editor/extensions/cachedCodeBlockLowlight.ts';

const paragraph = (text = '') => ({ type: 'paragraph', ...(text ? { content: [{ type: 'text', text }] } : {}) });
const code = (text = '') => ({ type: 'codeBlock', attrs: Object.assign(Object.create(null), { language: null }), ...(text ? { content: [{ type: 'text', text }] } : {}) });
function editor(content) {
  return new Editor({ extensions: [StarterKit.configure({ codeBlock: false }), CachedCodeBlockLowlight.configure({ lowlight: createLowlight() })], content: { type: 'doc', content } });
}

test('converts selected paragraphs to one code block and back, preserving blank lines', () => {
  const initial = [paragraph('First'), paragraph(), paragraph('Second'), paragraph()];
  const e = editor(initial);
  e.commands.selectAll();
  assert.equal(e.commands.toggleCodeBlock(), true);
  assert.deepEqual(e.getJSON().content, [code('First\n\nSecond\n')]);
  assert.equal(e.commands.toggleCodeBlock(), true);
  assert.deepEqual(e.getJSON().content, initial);
  e.destroy();
});

test('converts multiple existing code blocks back to paragraphs', () => {
  const e = editor([code('First'), code(), code('Second')]);
  e.commands.selectAll();
  e.commands.toggleCodeBlock();
  assert.deepEqual(e.getJSON().content, [paragraph('First'), paragraph(), paragraph('Second')]);
  e.destroy();
});

test('preserves hard breaks and leaves unselected blocks alone', () => {
  const first = { type: 'paragraph', content: [{ type: 'text', text: 'one' }, { type: 'hardBreak' }, { type: 'text', text: 'two' }] };
  const e = editor([first, paragraph('three'), paragraph('outside')]);
  const end = e.state.doc.child(0).nodeSize + e.state.doc.child(1).nodeSize - 1;
  e.commands.setTextSelection({ from: 1, to: end });
  e.commands.toggleCodeBlock();
  assert.deepEqual(e.getJSON().content, [code('one\ntwo\nthree'), paragraph('outside')]);
  e.destroy();
});

test('single cursor code toggle retains standard behaviour', () => {
  const e = editor([paragraph('Hello')]);
  e.commands.toggleCodeBlock();
  assert.deepEqual(e.getJSON().content, [code('Hello')]);
  e.commands.toggleCodeBlock();
  assert.deepEqual(e.getJSON().content, [paragraph('Hello')]);
  e.destroy();
});
