import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import MarkdownIt from 'markdown-it';
import { Schema } from '@tiptap/pm/model';
import { createServer, transformWithEsbuild } from 'vite';

const source = await readFile(
  new URL('../src/lib/editor/markdown.ts', import.meta.url),
  'utf8'
);
const { code } = await transformWithEsbuild(source, 'markdown.ts', {
  loader: 'ts',
  format: 'esm',
  target: 'esnext'
});
const { escapeBlockStarts, joinListItemBlocks, joinMarkdownBlocks, literalTextEscapes, serializeCodeBlock, serializeInlineMarkdown, serializeInlineMarkdownKeepingDollars } = await import(
  `data:text/javascript;base64,${Buffer.from(code).toString('base64')}`
);

const vite = await createServer({
  configFile: false,
  server: { middlewareMode: true },
  appType: 'custom'
});
let editorMarkdown;
try {
  const { createEditorMarkdownParser } = await vite.ssrLoadModule(
    new URL('../src/lib/editor/markdownParser.ts', import.meta.url).pathname
  );
  editorMarkdown = createEditorMarkdownParser();
} finally {
  await vite.close();
}

// Link intentionally precedes formatting marks in the schema. This matches the
// mark order produced by the app and reproduces the original per-text-node bug.
const schema = new Schema({
  nodes: {
    doc: { content: 'block+' },
    paragraph: { content: 'inline*', group: 'block' },
    text: { group: 'inline' },
    mathInline: { inline: true, group: 'inline', atom: true, attrs: { tex: { default: '' } } },
    image: {
      inline: true,
      group: 'inline',
      attrs: { src: {}, alt: { default: '' }, title: { default: null }, size: { default: 'full' }, width: { default: null }, height: { default: null } }
    }
  },
  marks: {
    link: { attrs: { href: {} } },
    bold: {},
    italic: {},
    code: {},
    wikiLink: {
      attrs: {
        title: { default: null },
        aliased: { default: false }
      }
    }
  }
});

const markdown = new MarkdownIt({ html: true, linkify: false, breaks: false });
const bold = schema.marks.bold.create();
const italic = schema.marks.italic.create();
const link = schema.marks.link.create({ href: 'https://example.com' });

function paragraph(segments) {
  return schema.node(
    'paragraph',
    null,
    segments.map(({ text, marks }) => schema.text(text, marks))
  );
}

function assertMarkdown(segments, expected, expectedHtml) {
  const serialized = serializeInlineMarkdown(paragraph(segments));
  assert.equal(serialized, expected);
  assert.equal(markdown.render(serialized), `${expectedHtml}\n`);
}

test('keeps bold open when a link ends the marked span', () => {
  assertMarkdown(
    [
      { text: 'Bold ', marks: [bold] },
      { text: 'link', marks: [link, bold] }
    ],
    '**Bold [link](https://example.com)**',
    '<p><strong>Bold <a href="https://example.com">link</a></strong></p>'
  );
});

test('keeps bold open when a link starts the marked span', () => {
  assertMarkdown(
    [
      { text: 'link', marks: [link, bold] },
      { text: ' bold', marks: [bold] }
    ],
    '**[link](https://example.com) bold**',
    '<p><strong><a href="https://example.com">link</a> bold</strong></p>'
  );
});

test('keeps one bold span around a link in the middle', () => {
  assertMarkdown(
    [
      { text: 'Before ', marks: [bold] },
      { text: 'link', marks: [link, bold] },
      { text: ' after', marks: [bold] }
    ],
    '**Before [link](https://example.com) after**',
    '<p><strong>Before <a href="https://example.com">link</a> after</strong></p>'
  );
});

test('keeps nested bold and italic marks open across a link', () => {
  assertMarkdown(
    [
      { text: 'Nested ', marks: [bold, italic] },
      { text: 'link', marks: [link, bold, italic] },
      { text: ' marks', marks: [bold, italic] }
    ],
    '***Nested [link](https://example.com) marks***',
    '<p><em><strong>Nested <a href="https://example.com">link</a> marks</strong></em></p>'
  );
});

test('preserves wiki-link aliases inside a bold span', () => {
  const wikiLink = schema.marks.wikiLink.create({ title: 'Target note', aliased: true });
  const serialized = serializeInlineMarkdown(
    paragraph([
      { text: 'See ', marks: [bold] },
      { text: 'this note', marks: [wikiLink, bold] },
      { text: ' now', marks: [bold] }
    ])
  );

  assert.equal(serialized, '**See [[Target note|this note]] now**');
});

test('round-trips inline code backticks, delimiter padding and literal whitespace', () => {
  const codeMark = schema.marks.code.create();
  for (const text of [
    'a`b',
    'a``b',
    'a````b',
    '`',
    '``',
    '`edge`',
    ' a ',
    ' leading',
    'trailing ',
    ' ',
    '   ',
    '\tleading',
    '\u2003leading',
    '\t\u2003  interior\t\u2003 ',
    '\\$x$'
  ]) {
    const serialized = serializeInlineMarkdown(paragraph([{ text, marks: [codeMark] }]));
    const tokens = editorMarkdown.parseInline(serialized, {})[0].children;
    assert.deepEqual(tokens.map((token) => [token.type, token.content]), [['code_inline', text]], text);
    assert.equal(editorMarkdown.render(serialized).includes('data-math-inline'), false, text);
  }
  assert.equal(serializeInlineMarkdown(paragraph([{ text: '`', marks: [codeMark] }])), '`` ` ``');
  assert.equal(serializeInlineMarkdown(paragraph([{ text: ' a ', marks: [codeMark] }])), '`  a  `');
  assert.equal(serializeInlineMarkdown(paragraph([{ text: '   ', marks: [codeMark] }])), '`   `');
});

test('joins contiguous code across formatting changes with code taking precedence', () => {
  const codeMark = schema.marks.code.create();
  const serialized = serializeInlineMarkdown(paragraph([
    { text: '`a', marks: [codeMark, bold] },
    { text: '``', marks: [codeMark, italic] },
    { text: 'b`', marks: [codeMark] }
  ]));
  assert.equal(serialized, '``` `a``b` ```');
  assert.deepEqual(
    editorMarkdown.parseInline(serialized, {})[0].children.map((token) => [token.type, token.content]),
    [['code_inline', '`a``b`']]
  );
  assert.equal(editorMarkdown.render(serialized), '<p data-helix-code=""><code style="white-space: pre-wrap">`a``b`</code></p>\n');
});

test('keeps link syntax outside code and joins code across formatting changes within links', () => {
  const codeMark = schema.marks.code.create();
  const serialized = serializeInlineMarkdown(paragraph([
    { text: 'before ', marks: [] },
    { text: '`a', marks: [codeMark, link, bold] },
    { text: 'b`', marks: [codeMark, link, italic] },
    { text: ' after', marks: [] }
  ]));
  assert.equal(serialized, 'before [`` `ab` ``](https://example.com) after');
  assert.equal(
    editorMarkdown.render(serialized),
    '<p data-helix-code="">before <a href="https://example.com"><code style="white-space: pre-wrap">`ab`</code></a> after</p>\n'
  );
});

test('keeps formatting that covers a whole code span', () => {
  const codeMark = schema.marks.code.create();
  assert.equal(serializeInlineMarkdown(paragraph([{ text: 'x.ps1', marks: [codeMark, bold] }])), '**`x.ps1`**');
  assert.equal(serializeInlineMarkdown(paragraph([{ text: 'x', marks: [codeMark, italic] }, { text: ' rest', marks: [] }])), '*`x`* rest');
  // Bold text running into bold code stays one bold run.
  assert.equal(serializeInlineMarkdown(paragraph([{ text: 'Run ', marks: [bold] }, { text: 'make', marks: [codeMark, bold] }, { text: ' now', marks: [bold] }])), '**Run `make` now**');
  // What is written loads back as bold code.
  assert.match(editorMarkdown.render('**`x.ps1`**'), /<strong><code[^>]*>x\.ps1<\/code><\/strong>/);
});

test('writes spaces at the edges of bold and italic outside the delimiters (issues #240, #276)', () => {
  const cases = [
    [[{ text: 'bold ', marks: [bold] }, { text: 'rest', marks: [] }], '**bold** rest', '<strong>bold</strong> rest'],
    [[{ text: 'pre', marks: [] }, { text: ' bold', marks: [bold] }, { text: ' rest', marks: [] }], 'pre **bold** rest', 'pre <strong>bold</strong> rest'],
    [[{ text: 'word ', marks: [italic] }, { text: '.', marks: [] }], '*word* .', '<em>word</em> .'],
    [[{ text: 'a ', marks: [] }, { text: 'bold ', marks: [bold] }, { text: 'it ', marks: [bold, italic] }, { text: 'end', marks: [] }], 'a **bold *it*** end', 'a <strong>bold <em>it</em></strong> end'],
    [[{ text: 'one ', marks: [bold] }, { text: 'two ', marks: [italic] }, { text: 'three', marks: [] }], '**one** *two* three', '<strong>one</strong> <em>two</em> three'],
    [[{ text: '  both  ', marks: [bold] }], '  **both**  ', '  <strong>both</strong>  '],
    // Spaces inside a formatted run stay inside it.
    [[{ text: 'a', marks: [bold] }, { text: ' ', marks: [bold] }, { text: 'b', marks: [bold] }], '**a b**', '<strong>a b</strong>'],
    // A mark over nothing but spaces is not written.
    [[{ text: ' ', marks: [bold] }, { text: 'x', marks: [] }], ' x', ' x'],
    [[{ text: 'link ', marks: [bold, link] }, { text: 'x', marks: [] }], '**[link](https://example.com)** x', '<strong><a href="https://example.com">link</a></strong> x'],
  ];
  for (const [segments, markdown, html] of cases) {
    assert.equal(serializeInlineMarkdown(paragraph(segments)), markdown);
    assert.equal(editorMarkdown.renderInline(markdown), html, markdown);
  }
});

test('drops formatting around a code span that a letter or digit touches', () => {
  const codeMark = schema.marks.code.create();
  // `**\`code\`**s` is not emphasis in Markdown, so the code span is written plain there.
  assert.equal(serializeInlineMarkdown(paragraph([{ text: 'code', marks: [codeMark, bold] }, { text: 's', marks: [] }])), '`code`s');
  assert.equal(serializeInlineMarkdown(paragraph([{ text: 'the', marks: [] }, { text: 'code', marks: [codeMark, bold] }, { text: ' x', marks: [] }])), 'the`code` x');
  assert.equal(serializeInlineMarkdown(paragraph([{ text: 'Run ', marks: [bold] }, { text: 'make', marks: [codeMark, bold] }, { text: 's', marks: [] }])), '**Run** `make`s');
  // Next to spaces or punctuation the formatting is kept.
  assert.equal(serializeInlineMarkdown(paragraph([{ text: '(', marks: [] }, { text: 'x', marks: [codeMark, bold] }, { text: ').', marks: [] }])), '(**`x`**).');
  assert.match(editorMarkdown.renderInline('(**`x`**).'), /<strong><code[^>]*>x<\/code><\/strong>/);
});

// What the editor does when it saves a paragraph: escape only what would read back differently.
const inlinePlainText = (markdown) => (editorMarkdown.parseInline(markdown, {})[0]?.children ?? [])
  .map((token) => (token.type === 'text' || token.type === 'code_inline' || token.type === 'helix_wiki' ? token.content : '')).join('');
const readsAsParagraph = (markdown) => {
  const tokens = editorMarkdown.parse(markdown, {});
  return tokens.length === 3 && tokens[0].type === 'paragraph_open';
};
function saveParagraph(segments) {
  const node = paragraph(segments);
  const literalText = literalTextEscapes(node, (source) => source, inlinePlainText);
  return escapeBlockStarts(serializeInlineMarkdown(node, (source) => source, { literalText }), readsAsParagraph);
}
/** Block types and visible text a saved paragraph has when it is loaded again. */
function reload(markdown) {
  const tokens = editorMarkdown.parse(markdown, {});
  return {
    blocks: tokens.filter((token) => token.nesting === 1).map((token) => token.type.replace('_open', '')).join(','),
    text: tokens.filter((token) => token.type === 'inline').flatMap((token) => token.children)
      .filter((token) => token.type === 'text' || token.type === 'code_inline').map((token) => token.content).join(''),
    formatted: tokens.filter((token) => token.type === 'inline').flatMap((token) => token.children)
      .some((token) => /^(em|strong|s|link|html_inline|mark)/.test(token.type)),
  };
}

test('text that looks like Markdown is saved so that it stays text', () => {
  const cases = [
    ['*not italic*', '\\*not italic*'],
    ['a*b*c in a formula', 'a\\*b*c in a formula'],
    ['__init__ is a method', '\\_\\_init__ is a method'],
    ['`not code`', '\\`not code`'],
    ['~~not struck~~', '\\~~not struck~~'],
    ['<b> shown as text', '\\<b> shown as text'],
    ['&amp; literal entity', '\\&amp; literal entity'],
    ['1. not a list', '1\\. not a list'],
    ['12) not a list', '12\\) not a list'],
    ['# not a heading', '\\# not a heading'],
    ['> not a quote', '\\> not a quote'],
    ['- not a bullet', '\\- not a bullet'],
    ['+ not a bullet', '\\+ not a bullet'],
    ['---', '\\---'],
    ['[x]: http://not.a/definition', '\\[x]: http://not.a/definition'],
    ['\\\\server\\share and C:\\.', '\\\\\\server\\share and C:\\\\.'],
  ];
  for (const [text, expected] of cases) {
    const saved = saveParagraph([{ text, marks: [] }]);
    assert.equal(saved, expected, text);
    assert.deepEqual(reload(saved), { blocks: 'paragraph', text, formatted: false }, saved);
    // Saving what was loaded again changes nothing more.
    assert.equal(saveParagraph([{ text: reload(saved).text, marks: [] }]), saved);
  }
});

test('text that reads as a link or an image is left to become one', () => {
  // Deliberate: images damaged by an old save bug (`<!-- -->![](a b.png)`) load as text and come
  // back as images this way. Brackets are never escaped.
  for (const text of ['![](.helixnotes/attachments/a b.png)', '[docs](https://example.com)', '[[Other note]]']) {
    assert.equal(saveParagraph([{ text, marks: [] }]), text);
  }
});

test('a line the note already has unescaped keeps its meaning from before', () => {
  // The editor shows a source line such as `    - item` as text; saving it as `- item` lets the
  // next load read the list its author wrote. Only text from elsewhere is escaped.
  const node = paragraph([{ text: '- item from an indented source line', marks: [] }]);
  const line = serializeInlineMarkdown(node, (source) => source);
  const source = new Set(['- item from an indented source line']);
  assert.equal(escapeBlockStarts(line, readsAsParagraph, (text) => source.has(text.trim())), '- item from an indented source line');
  assert.equal(escapeBlockStarts(line, readsAsParagraph, () => false), '\\- item from an indented source line');
  // Line by line: one known line does not excuse another.
  assert.equal(
    escapeBlockStarts('1. typed here\n2. from the source', readsAsParagraph, (text) => text.trim() === '2. from the source'),
    '1\\. typed here\n2. from the source'
  );
});

test('ordinary text is written exactly as before, without added backslashes', () => {
  for (const text of [
    '2 * 3 * 4', 'snake_case_name and file_name.md', 'price is [10] and a < b & c', 'R&D, AT&T, x = y, 5^2?',
    'a lone * star and a lone ` tick', 'TODO [ ] check', 'version 1.2.3 and #hashtag', 'C:\\Users\\tom\\notes',
    'e.g. 1.5x faster - really', 'Q&A: why? because!', 'a_b_c', '100% (approx.) ~ 3', 'email@example.com', '(a) [b] {c}',
  ]) {
    assert.equal(saveParagraph([{ text, marks: [] }]), text);
  }
});

test('literal text next to real formatting keeps both', () => {
  const codeMark = schema.marks.code.create();
  // Asterisks in plain text around bold text must not join the bold markers.
  let saved = saveParagraph([{ text: 'a*', marks: [] }, { text: 'b', marks: [bold] }, { text: '*c', marks: [] }]);
  assert.equal(reload(saved).text, 'a*b*c');
  assert.equal(editorMarkdown.renderInline(saved), 'a*<strong>b</strong>*c');
  // Text in a code span is never escaped.
  saved = saveParagraph([{ text: 'x ', marks: [] }, { text: 'a*b*c', marks: [codeMark] }, { text: ' and [docs]', marks: [] }]);
  assert.equal(saved, 'x `a*b*c` and [docs]');
  // A paragraph with formatting and harmless brackets gets no backslashes.
  saved = saveParagraph([{ text: 'Note:', marks: [bold] }, { text: ' see [docs] and file_name', marks: [] }]);
  assert.equal(saved, '**Note:** see [docs] and file_name');
});

test('preserves formatting on non-code parts of a link', () => {
  const codeMark = schema.marks.code.create();
  const serialized = serializeInlineMarkdown(paragraph([
    { text: 'first', marks: [link, bold] },
    { text: 'code', marks: [link, codeMark] },
    { text: 'last', marks: [link, bold] }
  ]));
  assert.equal(
    editorMarkdown.render(serialized),
    '<p data-helix-code=""><strong><a href="https://example.com">first</a></strong><a href="https://example.com"><code style="white-space: pre-wrap">code</code></a><strong><a href="https://example.com">last</a></strong></p>\n'
  );
});

test('round-trips code blocks with embedded fences and every blank-line boundary', () => {
  for (const text of [
    '',
    'plain',
    '\nleading',
    'trailing\n',
    '\n\nleading\n\nmiddle\n\ntrailing\n\n',
    '\t  \n\u2003\n\n',
    '```\nembedded\n```\n',
    '``````\n~~~\n$not math$\n```'
  ]) {
    const serialized = serializeCodeBlock(text, 'js');
    const [fence] = markdown.parse(serialized, {});
    const expectedFence = '`'.repeat(Math.max(3, ...[...text.matchAll(/`+/g)].map((match) => match[0].length + 1)));
    assert.equal(serialized, `${expectedFence}js\n${text}\n${expectedFence}\n`, text);
    assert.equal(fence.type, 'fence', text);
    assert.equal(fence.info, 'js', text);
    // CommonMark includes the structural newline before the closing fence.
    assert.equal(fence.content, `${text}\n`, text);
    assert.equal(fence.content.slice(0, -1), text);
    assert.equal(editorMarkdown.render(serialized).includes('data-math-inline'), false, text);
  }
  assert.equal(serializeCodeBlock('code'), '```\ncode\n```\n');
});

test('escapes literal prose dollars without changing preceding literal backslashes', () => {
  for (const text of [
    '$x$',
    '$$',
    'cost $5 and $10',
    '\\$x$',
    '\\\\$x$',
    '\\\\\\$x$',
    'before \\$x\\$ after',
    '$a$ and $b$',
    'literal $with spaces$',
    'ordinary punctuation: a+b, c=d; e/f!'
  ]) {
    const serialized = serializeInlineMarkdown(paragraph([{ text, marks: [] }]));
    const tokens = editorMarkdown.parseInline(serialized, {})[0].children;
    assert.equal(tokens.every((token) => token.type === 'text'), true, text);
    assert.equal(tokens.map((token) => token.content).join(''), text);
    assert.equal(editorMarkdown.render(serialized).includes('data-math-inline'), false, text);
  }
  assert.equal(serializeInlineMarkdown(paragraph([{ text: '$x$', marks: [] }])), '\\$x\\$');
  assert.equal(
    serializeInlineMarkdown(paragraph([{ text: 'ordinary punctuation: a+b, c=d; e/f!', marks: [] }])),
    'ordinary punctuation: a+b, c=d; e/f!'
  );
});

test('keeps escaped Markdown dollars as prose through load, save and reload', () => {
  for (const source of ['\\$x\\$', 'price \\$5', '\\\\\\$x\\$', '\\\\\\\\\\$x\\$']) {
    const loaded = editorMarkdown.parseInline(source, {})[0].children;
    assert.equal(loaded.every((token) => token.type === 'text'), true);
    const text = loaded.map((token) => token.content).join('');
    const serialized = serializeInlineMarkdown(paragraph([{ text, marks: [] }]));
    const reloaded = editorMarkdown.parseInline(serialized, {})[0].children;
    assert.equal(reloaded.every((token) => token.type === 'text'), true);
    assert.equal(reloaded.map((token) => token.content).join(''), text);
  }
});

test('preserves real inline math TeX and falls back to existing encoded HTML when needed', () => {
  for (const tex of [
    'x^2',
    '\\frac{a}{b}',
    '',
    '   ',
    ' leading',
    'trailing ',
    '\tleading',
    'trailing\u2003',
    'a\nb',
    'a\rb',
    'x$y',
    '\\$',
    'trailing\\'
  ]) {
    const mathNode = schema.node('mathInline', { tex });
    const serialized = serializeInlineMarkdown(schema.node('paragraph', null, [mathNode]));
    const html = editorMarkdown.render(serialized);
    const stored = html.match(/data-math-inline="([^"]*)"/);
    assert.ok(stored, tex);
    assert.equal(decodeURIComponent(stored[1]), tex);
    if (tex === 'x^2' || tex === '\\frac{a}{b}') {
      assert.equal(serialized, `$${tex}$`);
    } else {
      assert.equal(serialized, `<span data-math-inline="${encodeURIComponent(tex)}" class="math-inline"></span>`);
    }
    const reloaded = schema.node('paragraph', null, [
      schema.node('mathInline', { tex: decodeURIComponent(stored[1]) })
    ]);
    assert.equal(serializeInlineMarkdown(reloaded), serialized);
  }
});

test('keeps literal dollars escaped within rich formatting and link labels', () => {
  const serialized = serializeInlineMarkdown(paragraph([
    { text: 'before ', marks: [bold] },
    { text: '$literal$', marks: [bold, link] },
    { text: ' after', marks: [bold] }
  ]));
  assert.equal(serialized, '**before [\\$literal\\$](https://example.com) after**');
  assert.equal(
    editorMarkdown.render(serialized),
    '<p><strong>before <a href="https://example.com">$literal$</a> after</strong></p>\n'
  );
});

test('uses encoded math storage at ambiguous dollar, digit and adjacent-atom boundaries', () => {
  const mathNode = schema.node('mathInline', { tex: 'x' });
  for (const children of [
    [mathNode, schema.text('2')],
    [mathNode, schema.text('$literal$')],
    [schema.text('$literal$'), mathNode],
    [mathNode, schema.node('mathInline', { tex: 'y' })]
  ]) {
    const serialized = serializeInlineMarkdown(schema.node('paragraph', null, children));
    const stored = [...editorMarkdown.render(serialized).matchAll(/data-math-inline="([^"]*)"/g)];
    const expected = children.filter((child) => child.type.name === 'mathInline').map((child) => child.attrs.tex);
    assert.deepEqual(stored.map((match) => decodeURIComponent(match[1])), expected);
    assert.equal(serialized.startsWith('$x$'), false);
  }
});

test('preserves trailing prose backslashes before real math atoms', () => {
  for (const text of ['\\', '\\\\', '\\\\\\']) {
    const serialized = serializeInlineMarkdown(schema.node('paragraph', null, [
      schema.text(text),
      schema.node('mathInline', { tex: 'x' })
    ]));
    const tokens = editorMarkdown.parseInline(serialized, {})[0].children;
    assert.equal(tokens[0].type, 'text');
    assert.equal(tokens[0].content, text);
    const stored = editorMarkdown.render(serialized).match(/data-math-inline="([^"]*)"/);
    assert.ok(stored);
    assert.equal(decodeURIComponent(stored[1]), 'x');
  }
});

test('splits contiguous code at link boundaries without losing code text or link targets', () => {
  const codeMark = schema.marks.code.create();
  const otherLink = schema.marks.link.create({ href: 'https://other.example' });
  const serialized = serializeInlineMarkdown(paragraph([
    { text: '`first', marks: [codeMark] },
    { text: 'second`', marks: [codeMark, link] },
    { text: 'third', marks: [codeMark, otherLink] },
    { text: 'last', marks: [codeMark] }
  ]));
  assert.equal(
    editorMarkdown.render(serialized),
    '<p data-helix-code=""><code style="white-space: pre-wrap">`first</code><a href="https://example.com"><code style="white-space: pre-wrap">second`</code></a><a href="https://other.example"><code style="white-space: pre-wrap">third</code></a><code style="white-space: pre-wrap">last</code></p>\n'
  );
});

const block = (text) => ({ text, isImage: false });
const image = { text: '![x](a.png)\n', isImage: true };
const emptyParagraph = { text: '', isImage: false, isEmpty: true };

test('joined blocks are separated by exactly one blank line, including after lists', () => {
  assert.equal(joinMarkdownBlocks([block('one\n'), block('two\n')]), 'one\n\ntwo\n');
  assert.equal(joinMarkdownBlocks([block('- a\n- b\n\n'), block('text\n')]), '- a\n- b\n\ntext\n');
  assert.equal(joinMarkdownBlocks([block('- a\n\n'), block('1. b\n\n'), block('text\n')]), '- a\n\n1. b\n\ntext\n');
  assert.equal(joinMarkdownBlocks([block('text\n'), block('- a\n\n')]), 'text\n\n- a\n');
});

test('empty paragraphs stay as marker lines between blocks and are dropped at the end', () => {
  assert.equal(
    joinMarkdownBlocks([block('a\n'), emptyParagraph, emptyParagraph, block('b\n'), emptyParagraph, emptyParagraph]),
    'a\n\n<!-- -->\n<!-- -->\nb\n'
  );
  assert.equal(joinMarkdownBlocks([emptyParagraph]), '\n');
});

test('an empty paragraph before an image is written as a blank line that parses back to it', () => {
  const gap = '<p></p>\n<p><img';
  for (const before of [block('# T\n'), block('para\n'), block('- a\n\n'), block('```\ncode\n```\n')]) {
    const joined = joinMarkdownBlocks([before, emptyParagraph, image, block('text\n')]);
    assert.equal(joined, `${before.text.replace(/\n*$/, '\n')}\n![x](a.png)\n\ntext\n`, before.text);
    assert.doesNotMatch(joined, /<!--/);
    assert.ok(editorMarkdown.render(joined).includes(gap), joined);
    // Saving what was just loaded must not change it.
    assert.equal(joinMarkdownBlocks([before, emptyParagraph, image, block('text\n')]), joined);
  }
  // A second empty paragraph keeps its marker and still leaves a blank line before the image.
  const twice = joinMarkdownBlocks([block('para\n'), emptyParagraph, emptyParagraph, image]);
  assert.equal(twice, 'para\n\n<!-- -->\n\n![x](a.png)\n');
  assert.equal(editorMarkdown.render(twice).match(/<p><\/p>/g)?.length, 2);
});

test('an image placed directly after a block stays directly after it', () => {
  const joined = joinMarkdownBlocks([block('# T\n'), image]);
  assert.equal(joined, '# T\n![x](a.png)\n');
  assert.doesNotMatch(editorMarkdown.render(joined), /<p><\/p>/);
});

test('joining never collapses blank lines inside code blocks', () => {
  const code = serializeCodeBlock('a\n\n\n\nb');
  assert.equal(joinMarkdownBlocks([block(code), block('after\n')]), code + '\nafter\n');
  assert.ok(joinMarkdownBlocks([block(code), block('after\n')]).includes('a\n\n\n\nb'));
});

const mathPayloads = (markdown) => (editorMarkdown.parseInline(markdown, {})[0]?.children ?? [])
  .filter((token) => token.type === 'helix_math_inline').map((token) => token.content);
const keepDollars = (children) => serializeInlineMarkdownKeepingDollars(schema.node('paragraph', null, children), undefined, mathPayloads);
const mathAtom = (tex) => schema.node('mathInline', { tex });
// What a reader of the saved Markdown sees: plain text and formulas, in order.
const readBack = (markdown) => (editorMarkdown.parseInline(markdown, {})[0]?.children ?? []).map((token) =>
  token.type === 'helix_math_inline' ? `\u0000${token.content}\u0000` : token.type === 'text' ? token.content : '').join('');
const authored = (children) => children.map((child) => child.type.name === 'mathInline' ? `\u0000${child.attrs.tex}\u0000` : child.text).join('');

test('dollar signs that cannot form a formula are saved as typed', () => {
  for (const text of ['Cost $5 and $6 total', 'Price is $5.', 'Only $', '$ spaced $', 'a$b', '5$ and 6$', 'two $5 or $10']) {
    assert.equal(keepDollars([schema.text(text)]), text, text);
  }
});

test('dollar signs that would form a formula stay escaped, so literal text is never turned into math', () => {
  for (const text of ['$x$', 'cost $x$ here', '$a+b$', 'x $y$ z $w$', '$$', 'a $$ b']) {
    const saved = keepDollars([schema.text(text)]);
    assert.deepEqual(mathPayloads(saved), [], `${text} -> ${saved}`);
    assert.equal(readBack(saved), text, `${text} -> ${saved}`);
  }
});

test('real formulas keep their dollars and are not re-paired with neighbouring text', () => {
  const nodes = [schema.text('cost $5 and '), mathAtom('x'), schema.text(' then $6')];
  const saved = keepDollars(nodes);
  assert.deepEqual(mathPayloads(saved), ['x']);
  assert.equal(readBack(saved), authored(nodes));
  const pairing = [schema.text('$a'), mathAtom('x'), schema.text('b')];
  const pairingSaved = keepDollars(pairing);
  assert.deepEqual(mathPayloads(pairingSaved), ['x'], pairingSaved);
  assert.equal(readBack(pairingSaved), authored(pairing));
});

test('backslashes before an unescaped dollar are doubled so the dollar stays literal', () => {
  for (const text of ['a\\$5', 'a\\\\$5', 'path\\ $5 and $6']) {
    const saved = keepDollars([schema.text(text)]);
    assert.equal(readBack(saved), text, `${text} -> ${saved}`);
  }
});

test('fuzz: saving text and formulas keeps exactly what was written, whatever the dollars do', () => {
  const alphabet = ['a', 'b', '1', '5', ' ', '$', '$', '\\', '*', '_', '`', 'x', '.', '\n'.slice(0, 0) || ' '];
  let seed = 12345;
  const random = (n) => { seed = (seed * 1103515245 + 12345) & 0x7fffffff; return seed % n; };
  for (let round = 0; round < 4000; round++) {
    const children = [];
    for (let part = 0, count = 1 + random(4); part < count; part++) {
      if (random(5) === 0) children.push(mathAtom(['x', 'a+b', '5', 'y z'][random(4)]));
      else {
        const text = Array.from({ length: 1 + random(8) }, () => alphabet[random(alphabet.length)]).join('');
        if (text.length) children.push(schema.text(text));
      }
    }
    // Adjacent text nodes would merge; adjacent atoms are covered by the serializer's own encoding rules.
    const merged = children.reduce((list, child) => {
      const last = list[list.length - 1];
      if (last?.isText && child.isText) list[list.length - 1] = schema.text(last.text + child.text); else list.push(child);
      return list;
    }, []);
    if (!merged.length) continue;
    const saved = keepDollars(merged);
    const baseline = serializeInlineMarkdown(schema.node('paragraph', null, merged));
    assert.equal(readBack(saved), readBack(baseline), JSON.stringify({ saved, baseline, merged: authored(merged) }));
  }
});

const item = (text, isList = false) => ({ text, isList });

test('blocks after a list item\'s first paragraph stay inside the item, separated by a blank line', () => {
  assert.equal(joinListItemBlocks([item('only')]), 'only\n');
  // A nested list arrives ending in a blank line; it must not leave an indented blank line behind (issue #277).
  const list = (text) => ({ text, isList: true });
  assert.equal(joinListItemBlocks([item('parent'), list('- a\n- b\n\n')]), 'parent\n    - a\n    - b\n');
  assert.equal(joinListItemBlocks([item('parent'), list('- a\n    - deep\n\n')]), 'parent\n    - a\n        - deep\n');
  assert.equal(joinListItemBlocks([item('first'), item('second\n')]), 'first\n\n    second\n');
  assert.equal(
    joinListItemBlocks([item('step'), item('```sh\nrun\n\n\nagain\n```\n'), item('after\n')]),
    'step\n\n    ```sh\n    run\n\n\n    again\n    ```\n\n    after\n',
  );
  assert.equal(joinListItemBlocks([item('quoted'), item('> a\n>\n> b\n')]), 'quoted\n\n    > a\n    >\n    > b\n');
  assert.equal(joinListItemBlocks([item('first'), item('\n'), item('   \n'), item('real\n')]), 'first\n\n    real\n');
});

test('nested lists keep their direct four-space indentation under the item', () => {
  assert.equal(joinListItemBlocks([item('a'), item('- b\n', true)]), 'a\n    - b\n');
  assert.equal(joinListItemBlocks([item('a'), item('- b\n', true), item('```\nc\n```\n')]), 'a\n    - b\n\n    ```\n    c\n    ```\n');
});

test('joined list items parse back with their code, paragraphs and quotes still inside the item', () => {
  for (const marker of ['- ', '1. ', '10. ', '- [ ] ']) {
    const text = marker + joinListItemBlocks([item('step'), item('```js\nconst a = 1;\n\nconst b = 2;\n```\n'), item('more text\n'), item('> quoted\n')]);
    const html = editorMarkdown.render(text + '- next\n');
    const listItem = html.match(/<li[^>]*>[\s\S]*?<\/li>/)[0];
    assert.match(listItem, /<pre><code[^>]*>const a = 1;\n\nconst b = 2;<\/code><\/pre>/, marker);
    assert.match(listItem, /<p>more text<\/p>/, marker);
    assert.match(listItem, /<blockquote>\s*<p>quoted<\/p>/, marker);
    assert.equal(html.match(/<li[ >]/g).length, 2, `${marker}: the following item is a separate item`);
  }
});

test('joining a very large note takes linear time', () => {
  const blocks = Array.from({ length: 30000 }, (_, index) => block(`Paragraph ${index} with some words in it.\n`));
  for (let index = 0; index < blocks.length; index += 200) blocks[index] = block('- a\n- b\n\n');
  const started = performance.now();
  const joined = joinMarkdownBlocks(blocks);
  assert.ok(performance.now() - started < 500, 'building the note must not re-read the growing text for every block');
  assert.ok(joined.startsWith('- a\n- b\n\nParagraph 1 with'));
  assert.equal(joined.endsWith('Paragraph 29999 with some words in it.\n'), true);
  assert.ok(!joined.includes('\n\n\n'));
});

test('an image keeps the link around it, alone or within text', () => {
  const linked = (attrs, href = 'https://example.com') =>
    schema.nodes.image.create(attrs, null, [schema.marks.link.create({ href })]);
  const alone = schema.node('paragraph', null, [linked({ src: 'logo.png' })]);
  assert.equal(serializeInlineMarkdown(alone), '[![](logo.png)](https://example.com)');
  assert.match(markdown.render(serializeInlineMarkdown(alone)), /<a href="https:\/\/example.com"><img src="logo.png"/);
  const within = schema.node('paragraph', null, [
    schema.text('See '), linked({ src: 'badge.svg', alt: 'badge' }, 'https://ci.example.com'), schema.text(' here')
  ]);
  assert.equal(serializeInlineMarkdown(within), 'See [![badge](badge.svg)](https://ci.example.com) here');
  // Without a link an image still goes on its own line, as before.
  const plain = schema.node('paragraph', null, [schema.text('Text '), schema.nodes.image.create({ src: 'a.png', alt: 'alt' })]);
  assert.equal(serializeInlineMarkdown(plain), 'Text \n![alt](a.png)');
});

test('an image with a width or height is written as HTML so the size is kept', () => {
  const sized = schema.node('paragraph', null, [schema.nodes.image.create({ src: 'd.png', alt: 'Logo "x"', width: '120', height: '40' })]);
  const html = serializeInlineMarkdown(sized);
  assert.equal(html, '<img src="d.png" alt="Logo &quot;x&quot;" width="120" height="40">');
  assert.match(markdown.render(html), /<img src="d.png" alt="Logo &quot;x&quot;" width="120" height="40">/);
});
