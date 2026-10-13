import assert from 'node:assert/strict';
import test from 'node:test';
import { Schema } from '@tiptap/pm/model';
import { EditorState, TextSelection } from '@tiptap/pm/state';
import { history, undo, redo } from '@tiptap/pm/history';
import { common, createLowlight } from 'lowlight';
import { createCodeBlockHighlightPlugin } from '../src/lib/editor/extensions/cachedCodeBlockLowlight.ts';

const schema = new Schema({
  nodes: {
    doc: { content: 'block+' },
    paragraph: { content: 'text*', group: 'block' },
    blockquote: { content: 'block+', group: 'block' },
    codeBlock: { content: 'text*', group: 'block', code: true, attrs: { language: { default: null } } },
    text: { group: 'inline' },
  },
});

function codeBlock(text, language = 'javascript') {
  return schema.node('codeBlock', { language }, text ? schema.text(text) : null);
}

function setup(blocks, defaultLanguage = 'text') {
  const lowlight = createLowlight(common);
  const calls = [];
  const plugin = createCodeBlockHighlightPlugin({
    defaultLanguage,
    lowlight: {
      ...lowlight,
      highlight(language, text) {
        calls.push({ language, text });
        return lowlight.highlight(language, text);
      },
      highlightAuto(text) {
        calls.push({ language: 'auto', text });
        return lowlight.highlightAuto(text);
      },
    },
  });
  const state = EditorState.create({ schema, doc: schema.node('doc', null, blocks), plugins: [history(), plugin] });
  return { state, plugin, calls };
}

function keywordRanges(plugin, state) {
  return plugin.getState(state).decorations.find()
    .filter((decoration) => decoration.type.attrs.class.split(' ').includes('hljs-keyword'))
    .map(({ from, to }) => ({ from, to, text: state.doc.textBetween(from, to) }));
}

function expectedKeywords(doc) {
  const ranges = [];
  doc.descendants((node, pos) => {
    if (node.type.name === 'codeBlock' && node.attrs.language === 'javascript') {
      const offset = node.textContent.indexOf('const');
      if (offset >= 0) ranges.push({ from: pos + 1 + offset, to: pos + 6 + offset, text: 'const' });
    }
  });
  return ranges;
}

test('typing highlights only the changed block; selection-only transactions reuse plugin state', () => {
  const fixture = setup(Array.from({ length: 80 }, (_, i) => codeBlock(`const item${i} = 1;`)));
  let { state } = fixture;
  const { plugin, calls } = fixture;
  assert.equal(calls.length, 80);
  calls.length = 0;
  state = state.apply(state.tr.insertText('2', 1 + 'const item0 = 1'.length));
  assert.deepEqual(calls, [{ language: 'javascript', text: 'const item0 = 12;' }]);
  assert.deepEqual(keywordRanges(plugin, state), expectedKeywords(state.doc));
  calls.length = 0;
  const beforeSelection = plugin.getState(state);
  state = state.apply(state.tr.setSelection(TextSelection.create(state.doc, state.doc.content.size - 1)));
  assert.equal(plugin.getState(state), beforeSelection);
  assert.deepEqual(calls, []);
});

test('insertions, moves, nested blocks, and deletions keep cached spans at their new positions', () => {
  const first = codeBlock('const first = 1;');
  const second = codeBlock('const second = 2;');
  const fixture = setup([first, schema.node('blockquote', null, [second])]);
  let { state } = fixture;
  const { plugin, calls } = fixture;
  calls.length = 0;
  const prefix = schema.node('paragraph', null, schema.text('Inserted before code'));
  state = state.apply(state.tr.insert(0, prefix));
  assert.deepEqual(keywordRanges(plugin, state), expectedKeywords(state.doc));
  const move = state.tr.delete(prefix.nodeSize, prefix.nodeSize + first.nodeSize);
  move.insert(move.doc.content.size, first);
  state = state.apply(move);
  assert.deepEqual(keywordRanges(plugin, state), expectedKeywords(state.doc));
  state = state.apply(state.tr.delete(prefix.nodeSize, prefix.nodeSize + second.nodeSize + 2));
  assert.deepEqual(keywordRanges(plugin, state), expectedKeywords(state.doc));
  assert.deepEqual(calls, []);
});

test('language changes, code-to-paragraph conversion, and full replacement discard obsolete decorations', () => {
  const original = codeBlock('const value = 1;');
  const fixture = setup([original, codeBlock('const other = 2;')]);
  let { state } = fixture;
  const { plugin, calls } = fixture;
  calls.length = 0;
  state = state.apply(state.tr.setNodeMarkup(0, undefined, { language: 'text' }));
  assert.deepEqual(calls, [{ language: 'text', text: original.textContent }]);
  assert.deepEqual(keywordRanges(plugin, state), expectedKeywords(state.doc));
  calls.length = 0;
  state = state.apply(state.tr.setNodeMarkup(original.nodeSize, schema.nodes.paragraph));
  assert.deepEqual(plugin.getState(state).decorations.find(), []);
  assert.deepEqual(calls, []);
  state = state.apply(state.tr.replaceWith(0, state.doc.content.size, original));
  assert.deepEqual(keywordRanges(plugin, state), expectedKeywords(state.doc));
  assert.deepEqual(calls, [], 'reused immutable nodes retain their relative highlighting');
});

test('undo and redo restore token ranges without highlighting unrelated blocks', () => {
  const fixture = setup([codeBlock('const first = 1;'), codeBlock('const second = 2;')]);
  let { state } = fixture;
  const { plugin, calls } = fixture;
  const before = state.doc;
  state = state.apply(state.tr.insertText('/*new*/ ', 1));
  const after = state.doc;
  calls.length = 0;
  assert.equal(undo(state, (transaction) => { state = state.apply(transaction); }), true);
  assert.ok(state.doc.eq(before));
  assert.deepEqual(keywordRanges(plugin, state), expectedKeywords(state.doc));
  assert.ok(calls.length <= 1);
  assert.ok(calls.every(({ text }) => text === 'const first = 1;'));
  calls.length = 0;
  assert.equal(redo(state, (transaction) => { state = state.apply(transaction); }), true);
  assert.ok(state.doc.eq(after));
  assert.deepEqual(keywordRanges(plugin, state), expectedKeywords(state.doc));
  assert.ok(calls.length <= 1);
  assert.ok(calls.every(({ text }) => text === '/*new*/ const first = 1;'));
});

test('cache ownership respects highlighter/default-language boundaries and empty blocks', () => {
  const shared = codeBlock('const value = 1;', null);
  const javascript = setup([shared], 'javascript');
  const plaintext = setup([shared], 'text');
  assert.equal(javascript.plugin.getState(javascript.state).decorations.find()[0].type.attrs.class, 'hljs-keyword');
  assert.deepEqual(plaintext.plugin.getState(plaintext.state).decorations.find(), []);
  const empty = codeBlock('');
  const fixture = setup([empty]);
  let { state } = fixture;
  fixture.calls.length = 0;
  state = state.apply(state.tr.insert(0, schema.node('paragraph')));
  assert.deepEqual(fixture.calls, []);
  state = state.apply(state.tr.insertText('const x = 1;', 3));
  assert.deepEqual(fixture.calls, [{ language: 'javascript', text: 'const x = 1;' }]);
  assert.deepEqual(keywordRanges(fixture.plugin, state), expectedKeywords(state.doc));
});

test('a named language without a grammar is left plain instead of guessed', () => {
  const { plugin, state, calls } = setup([codeBlock('graph TD; A-->B;', 'mermaid'), codeBlock('const a = 1;', null)], null);
  assert.deepEqual(calls, [{ language: 'auto', text: 'const a = 1;' }]);
  assert.equal(plugin.getState(state).decorations.find(0, 'graph TD; A-->B;'.length + 2).length, 0);
});
