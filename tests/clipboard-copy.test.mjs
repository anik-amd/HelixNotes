import assert from 'node:assert/strict';
import test from 'node:test';
import { applySingleLineSpacing } from '../src/lib/utils/clipboard-copy.ts';

const STYLE = 'line-height:100%;margin:0';

test('paragraphs, headings, list items and quotes are copied with single spacing', () => {
  assert.equal(
    applySingleLineSpacing('<p>One</p><p>Two</p>'),
    `<p style="${STYLE}">One</p><p style="${STYLE}">Two</p>`
  );
  assert.equal(
    applySingleLineSpacing('<h1>Title</h1><ul><li>Item</li></ul><blockquote><p>Quote</p></blockquote>'),
    `<h1 style="${STYLE}">Title</h1><ul><li style="${STYLE}">Item</li></ul><blockquote style="line-height:100%"><p style="${STYLE}">Quote</p></blockquote>`
  );
});

test('existing inline styles are kept and the copy style wins', () => {
  assert.equal(
    applySingleLineSpacing('<p style="text-align: center; line-height: 1.5">Centered</p>'),
    `<p style="text-align: center; line-height: 1.5;${STYLE}">Centered</p>`
  );
  assert.equal(
    applySingleLineSpacing("<p class='lead' style='margin-bottom: 8px'>Lead</p>"),
    `<p class='lead' style='margin-bottom: 8px;${STYLE}'>Lead</p>`
  );
  assert.equal(
    applySingleLineSpacing('<p style="">Empty</p>'),
    `<p style="${STYLE}">Empty</p>`
  );
});

test('non-block markup is untouched', () => {
  const html = '<pre><code>&lt;p&gt;not a block&lt;/p&gt;</code></pre><p><strong>bold</strong> <a href="https://example.com">link</a></p><table><tr><td>cell</td></tr></table><path d="M0 0"/>';
  assert.equal(
    applySingleLineSpacing(html),
    `<pre><code>&lt;p&gt;not a block&lt;/p&gt;</code></pre><p style="${STYLE}"><strong>bold</strong> <a href="https://example.com">link</a></p><table><tr><td>cell</td></tr></table><path d="M0 0"/>`
  );
});
