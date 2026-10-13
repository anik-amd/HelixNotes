import assert from 'node:assert/strict';
import test from 'node:test';

const { decodeBuiltinNotebookIcon, decodeNotebookIcon, encodeBuiltinNotebookIcon } = await import(
  new URL('./notebook-icons.ts', import.meta.url)
);

test('encodes a builtin notebook icon with an optional color', () => {
  assert.equal(encodeBuiltinNotebookIcon('star'), 'builtin:star');
  assert.equal(encodeBuiltinNotebookIcon('star', null), 'builtin:star');
  assert.equal(encodeBuiltinNotebookIcon('star', '#E11D48'), 'builtin:star#e11d48');
  assert.equal(encodeBuiltinNotebookIcon('star', 'red'), 'builtin:star');
});

test('decodes notebook icons with and without a color', () => {
  assert.deepEqual(decodeNotebookIcon('builtin:star'), { icon: 'star', color: null });
  assert.deepEqual(decodeNotebookIcon('builtin:star#e11d48'), { icon: 'star', color: '#e11d48' });
  assert.deepEqual(decodeNotebookIcon('builtin:star#nope'), { icon: 'star', color: null });
});

test('ignores custom images and unknown builtin icons', () => {
  assert.equal(decodeNotebookIcon(null), null);
  assert.equal(decodeNotebookIcon('.helixnotes/attachments/icon#1.png'), null);
  assert.equal(decodeNotebookIcon('builtin:unknown#e11d48'), null);
});

test('tag icons stay uncolored builtin ids', () => {
  assert.equal(decodeBuiltinNotebookIcon('builtin:star'), 'star');
  assert.equal(decodeBuiltinNotebookIcon('builtin:star#e11d48'), null);
});
