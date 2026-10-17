import assert from 'node:assert/strict';
import test from 'node:test';

const { countWords, markdownCountText } = await import(new URL('./word-count.ts', import.meta.url));

test('counts words, not punctuation or table borders', () => {
  assert.equal(countWords(''), 0);
  assert.equal(countWords('  one two\nthree  '), 3);
  assert.equal(countWords('| a | b |'), 2);
  assert.equal(countWords('- > --- |'), 0);
  assert.equal(countWords("it's e-mail, 2026 café"), 4);
});

test('a table counts its cell words in source mode', () => {
  const md = '| Name | Role |\n| --- | :---: |\n| Ada Lovelace | First programmer |\n';
  assert.equal(countWords(markdownCountText(md)), 6);
});

test('Markdown syntax does not count as words', () => {
  const md = '---\ntitle: X\n---\n# Title here\n\n- [ ] task one\n- [x] done\n1. first\n  - nested **bold** item\n> quoted text\n\n[[Note|shown name]] [link](https://x.com) ![img](a.png) `code`\n';
  assert.equal(countWords(markdownCountText(md)), 15);
});
