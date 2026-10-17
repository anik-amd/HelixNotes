import assert from 'node:assert/strict';
import test from 'node:test';

const { htmlHasFormatting, looksLikeMarkdown } = await import(new URL('./markdown-paste.ts', import.meta.url));

test('Markdown block and inline syntax is recognized', () => {
  for (const text of [
    '# Title\n\nText', 'Intro\n\n- one\n- two', '1. first\n2. second', '> quoted', '```js\ncode\n```',
    '| a | b |\n| --- | --- |', 'Some **bold** text', 'see `code` here', '[label](https://x.com)', '![](img.png)',
    '[[Note]] link', 'an *emphasis* word', '- [ ] task', '---',
  ]) assert.equal(looksLikeMarkdown(text), true, text);
});

test('plain text stays plain', () => {
  for (const text of [
    '', 'Just a sentence.', 'Two\nlines of text', 'https://example.com/page', '2 * 3 * 4 = 24', 'price: $5 - $10',
    'snake_case_name and another_one', 'a # not a heading', '-1 degrees', 'C:\\Users\\me\\file.txt',
  ]) assert.equal(looksLikeMarkdown(text), false, text);
});

test('styled text from a code editor is not formatting', () => {
  assert.equal(htmlHasFormatting('<meta charset="utf-8"><div style="color: #ccc;font-family: monospace;white-space: pre;"><div><span style="color: #569cd6;"># Title</span></div></div>'), false);
  assert.equal(htmlHasFormatting('<p>Text with <strong>bold</strong></p>'), true);
  assert.equal(htmlHasFormatting('<ul><li>item</li></ul>'), true);
  assert.equal(htmlHasFormatting('<a href="x">x</a>'), true);
});
