import assert from 'node:assert/strict';
import test from 'node:test';
import { createEditorMarkdownParser, displayMathRanges, stripLinkTitle } from '../src/lib/editor/markdownParser.ts';

const parser = createEditorMarkdownParser();
const payload = '$x$\n$$\na+b\n$$\n[[Note]]\n[label](path with spaces)\n![image](<image path> "title")\n- [x] task\n<!-- -->\n<div data-pdf-src="a.pdf" data-pdf-name="A"></div>';
const escapedCode = (html) => html.match(/<pre><code[^>]*>([\s\S]*?)<\/code><\/pre>/)?.[1];
const unescape = (html) => html.replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&amp;/g, '&');
const mathPayloads = (html) => [...html.matchAll(/data-math-(?:inline|block)="([^"]*)"/g)].map(match => decodeURIComponent(unescape(match[1])));

for (const [name, opening, closing] of [
  ['backtick', '```text', '```'], ['tilde', '~~~text', '~~~'],
  ['long backtick', '````text', '````'], ['indented fence', '   ~~~text', '   ~~~'],
]) {
  test(`${name} fences preserve all literal syntax`, () => {
    const html = parser.render(`${opening}\n${payload}\n${closing}`, { wikiLinkTitles: [{ path: 'Note.md', title: 'Note' }] });
    assert.equal(unescape(escapedCode(html)), payload);
    assert.equal(mathPayloads(html).length, 0);
    assert.doesNotMatch(html, /<span[^>]*data-wiki-link|<li[^>]*data-checked|<div[^>]*data-secret-block/);
  });
}

test('shorter and mismatched fences remain code; following math still renders', () => {
  const code = '```\n~~~\n$x$';
  const html = parser.render('````text\n' + code + '\n````\n\nActual $y$.');
  assert.equal(unescape(escapedCode(html)), code);
  assert.deepEqual(mathPayloads(html), ['y']);
});

test('fences interrupt prose without needing a blank line', () => {
  const html = parser.render('Prose\n```text\n$x$\n```\nMath $y$.');
  assert.equal(unescape(escapedCode(html)), '$x$');
  assert.deepEqual(mathPayloads(html), ['y']);
});

test('quoted and listed fences protect their contents', () => {
  const quoted = parser.render('> ~~~text\n> $x$\n> [[Note]]\n> ~~~', { wikiLinkTitles: [] });
  assert.equal(unescape(escapedCode(quoted)), '$x$\n[[Note]]');
  const listed = parser.render('- item\n\n  ```text\n  $x$\n  <!-- -->\n  ```');
  assert.equal(unescape(escapedCode(listed)), '$x$\n<!-- -->');
});

test('unclosed fences keep their payload literal', () => {
  const html = parser.render('~~~text\n$x$\n<!-- -->');
  assert.equal(unescape(escapedCode(html)), '$x$\n<!-- -->');
  assert.deepEqual(mathPayloads(html), []);
});

test('fence rendering preserves leading, internal and extra trailing blank lines', () => {
  const code = '\nfirst\n\n\nlast\n\n';
  const html = parser.render('```text\n' + code + '\n```');
  assert.equal(unescape(escapedCode(html)), code);
});

test('inline and multiline code protect math, wiki and legacy links', () => {
  const html = parser.render('`` $x$ [[Note]] [a](a b) ` `` and `line\n$x$`', { wikiLinkTitles: [] });
  assert.match(html, /<code[^>]*>\$x\$ \[\[Note\]\] \[a\]\(a b\) `<\/code>/);
  assert.match(html, /<code[^>]*>line \$x\$<\/code>/);
  assert.deepEqual(mathPayloads(html), []);
});

test('escaped dollars stay prose while real math retains exact escaped-dollar and backtick TeX', () => {
  const html = parser.render('Escaped \\$x$ and $a\\$b$ and $`raw`$; $10 to $50 and $ x$ stay prose.');
  assert.deepEqual(mathPayloads(html), ['a\\$b', '`raw`']);
  assert.match(html, /Escaped \$x\$/);
  assert.match(html, /\$10 to \$50/);
});

test('math payloads are consumed before wiki, link and task transformations', () => {
  const tex = '[[Note]]\n[label](path with spaces)\n```\n- [x] TeX';
  const html = parser.render('$$\n' + tex + '\n$$\n\n$x+y$', { wikiLinkTitles: [] });
  assert.deepEqual(mathPayloads(html), [tex, 'x+y']);
  assert.doesNotMatch(html, /data-wiki-link|data-checked|<pre>/);
});

test('unclosed display math stays literal and normal fenced indentation convention stays unchanged', () => {
  assert.deepEqual(mathPayloads(parser.render('$$\nunclosed')), []);
  assert.doesNotMatch(parser.render('\tordinary tab-indented text'), /<pre>/);
});

test('semantic math ranges do not mistake fenced or inline code for display math', () => {
  const source = '~~~text\n$$\n# code\n$$\n~~~\n\n> $$\n> x\n> ---\n> $$\n\n# Real';
  const ranges = displayMathRanges(source);
  assert.equal(ranges.length, 1);
  assert.equal(source.slice(ranges[0].from, ranges[0].to), '> $$\n> x\n> ---\n> $$\n');
  assert.deepEqual(displayMathRanges('```text\n$$\n```\n\n# Real'), []);
});

test('wiki lookup preserves path precedence, shallowest title and escaped attributes', () => {
  const env = { wikiLinkTitles: [
    { path: 'deep/nested/Note.md', title: 'Note' }, { path: 'folder/Note.md', title: 'NOTE' },
    { path: 'other/Note.md', title: 'Note' }, { path: 'Quoted".md', title: 'Quoted"' },
  ] };
  const html = parser.render('[[Note]] [[deep/nested/Note#Heading|Alias]] [[Quoted"]] [[Missing]]', env);
  assert.match(html, /data-path="folder\/Note\.md"/);
  assert.match(html, /data-path="deep\/nested\/Note\.md" data-title="deep\/nested\/Note#Heading"[^>]*>Alias<\/span>/);
  assert.match(html, /data-path="Quoted&quot;\.md" data-title="Quoted&quot;"/);
  assert.match(html, /data-path="" data-title="Missing"/);
  assert.doesNotMatch(parser.render('\\[[Note]]', env), /data-wiki-link/);
  assert.doesNotMatch(parser.render('[[Note]]'), /data-wiki-link/);
});

test('wiki lookup reads titles once per titles array and refreshes when the array is replaced', () => {
  let reads = 0;
  const notes = Array.from({ length: 250 }, (_, index) => ({
    path: `folder/Note ${index}.md`, get title() { reads++; return `Note ${index}`; },
  }));
  const env = { wikiLinkTitles: notes };
  const source = '[[Note 249]] '.repeat(100);
  parser.render(source, env);
  parser.render(source, env);
  assert.equal(reads, notes.length);
  const updated = notes.slice();
  updated[249] = { path: 'Moved.md', title: 'Note 249' };
  assert.match(parser.render(source, { wikiLinkTitles: updated }), /data-path="Moved\.md"/);
});

test('ordinary and legacy links/images remain functional while unsafe schemes remain rejected', () => {
  const html = parser.render('[a](<path with spaces> "title") [b](path with spaces) [c](path%20"title") ![d](<image path>%20"title") ![e](///home/user/image.png)');
  assert.match(html, /href="path%20with%20spaces"/);
  assert.match(html, /href="path"/);
  assert.match(html, /src="image%20path"/);
  assert.match(html, /src="\/home\/user\/image\.png"/);
  // Links and images keep their titles, legacy destinations included; a link without one gets none.
  assert.match(html, /<a href="path%20with%20spaces" title="title">a<\/a>/);
  assert.match(html, /<a href="path%20with%20spaces">b<\/a>/);
  assert.match(html, /<a href="path" title="title">c<\/a>/);
  assert.match(html, /<img src="image%20path" alt="d" title="title">/);
  assert.match(html, /<img src="\/home\/user\/image\.png" alt="e">/);
  assert.doesNotMatch(parser.render('[bad](javascript:alert with spaces) ![bad](data:text/html;base64,foo bar)'), /href=|src=/);
});

test('compatibility destinations retain quoted parentheses and native nested URLs', () => {
  const html = parser.render('[a](path with spaces "a (title)") [b](<image path> (title)) [c](path(one))');
  assert.match(html, /href="path%20with%20spaces"/);
  assert.match(html, /href="image%20path"/);
  assert.match(html, /href="path\(one\)"/);
});

test('task and empty-paragraph handling uses real block structure, including nested and empty tasks', () => {
  const html = parser.render('- [x] **done**\n- [ ]\n  - [ ] nested\n\n<!-- -->\n# Next');
  assert.equal([...html.matchAll(/data-type="taskList"/g)].length, 2);
  assert.equal([...html.matchAll(/data-type="taskItem"/g)].length, 3);
  assert.match(html, /data-checked="true"[^>]*>[\s\S]*<strong>done<\/strong>/);
  assert.match(html, /<p><\/p>\n<h1>Next<\/h1>/);
  assert.doesNotMatch(parser.render('`- [x] not a task` and `<!-- -->`'), /data-checked|<p><\/p>/);
});

test('image-only gaps remain outside code and do not alter inline images', () => {
  assert.match(parser.render('Text\n\n![a](image.png)'), /<p>Text<\/p>\n<p><\/p>\n<p><img/);
  assert.doesNotMatch(parser.render('Text ![a](image.png)'), /<p><\/p>/);
});

test('only actual closed secret fences become secret blocks, not literal examples', () => {
  assert.match(parser.render('```helix-secret\nexample-payload\n```'), /data-secret-block="example-payload"/);
  const literal = '```helix-secret\nexample-payload\n```';
  assert.equal(unescape(escapedCode(parser.render('````text\n' + literal + '\n````'))), literal);
  assert.doesNotMatch(parser.render('```helix-secret\nunclosed'), /data-secret-block/);
});

test('invalid final secret-fence markers remain literal code', () => {
  for (const closing of ['    ```', '\t```', '```\u00a0']) {
    const html = parser.render('```helix-secret\nexample-payload\n' + closing);
    assert.doesNotMatch(html, /data-secret-block/);
    assert.equal(unescape(escapedCode(html)), 'example-payload\n' + closing);
  }
});

test('code whitespace is marked for DOM import without changing soft or hard line-break semantics', () => {
  const html = parser.render('# `   `\n\nbefore `trailing `\nnext  \nlast');
  assert.match(html, /<h1 data-helix-code="">/);
  assert.match(html, /<p data-helix-code="">/);
  assert.match(html, /<\/code> next<br>last/);
});

test('a callout keeps a newline between its title line and body, while ordinary soft breaks stay spaces', () => {
  assert.match(parser.render('> [!warning] Heads up\n> body here\n> more'), /\[!warning\] Heads up\nbody here more/);
  assert.match(parser.render('> [!note]\n> body\n> more'), /\[!note\]\nbody more/);
  assert.match(parser.render('> [!todo] Outer\n> > [!tip] Inner\n> > body'), /\[!todo\] Outer<\/p>[\s\S]*\[!tip\] Inner\nbody/);
  assert.match(parser.render('> > [!tip] Inner\n> > body'), /\[!tip\] Inner\nbody/);
  assert.match(parser.render('> plain\n> quote'), /plain quote/);
  assert.match(parser.render('one\ntwo'), /one two/);
  assert.match(parser.render('> [!tip] Title only'), /\[!tip\] Title only<\/p>/);
});

test('task markers only turn bullet-list items into tasks, never numbered items', () => {
  const ordered = parser.render('1. [x] done\n2. [ ] todo\n3. plain');
  assert.doesNotMatch(ordered, /data-type|data-checked/);
  assert.match(ordered, /<li>\[x\] done<\/li>\s*<li>\[ \] todo<\/li>/);
  const bullets = parser.render('- [x] done\n- [ ] todo');
  assert.equal([...bullets.matchAll(/data-type="taskItem"/g)].length, 2);
  assert.match(bullets, /<ul data-type="taskList">/);
  const nested = parser.render('1. one\n   - [x] nested task');
  assert.equal([...nested.matchAll(/data-type="taskItem"/g)].length, 1);
  assert.doesNotMatch(nested, /<ol[^>]*data-type/);
});

test('code spans mark simple list items and table cells so whitespace-only code survives DOM import', () => {
  assert.match(parser.render('- a `  ` b\n- plain'), /<li data-helix-code="">a <code[^>]*>  <\/code> b<\/li>\s*<li>plain<\/li>/);
  assert.match(parser.render('1. one `  ` two'), /<li data-helix-code="">one /);
  assert.match(parser.render('- [ ] task `  ` x'), /<li data-type="taskItem" data-checked="false" data-helix-code="">/);
  assert.match(parser.render('| a | b |\n| - | - |\n| `  ` | c |'), /<td data-helix-code=""><code[^>]*>  <\/code><\/td>\s*<td>c<\/td>/);
  assert.match(parser.render('| `  ` | b |\n| - | - |\n| x | y |'), /<th data-helix-code="">/);
});

test('list items with nested blocks are not marked, so no stray whitespace text appears around them', () => {
  const nested = parser.render('- a `x`\n  - nested `y`');
  assert.match(nested, /<li>a <code/);
  assert.match(nested, /<li data-helix-code="">nested /);
  assert.doesNotMatch(parser.render('- item `x`\n\n  ```js\n  code\n  ```'), /<li data-helix-code/);
  assert.doesNotMatch(parser.render('- no code here\n- still none'), /data-helix-code/);
  // Loose lists keep their paragraph marker as before.
  assert.match(parser.render('- loose `  ` x\n\n- second'), /<li>\s*<p data-helix-code="">loose /);
});

test('stripLinkTitle behaves exactly like the title expression it replaced, on every short combination', () => {
  const oracle = (destination) => destination.replace(/(?:\s|%20)+(?:"[^"]*"|'[^']*')(?:\s|%20)*$/u, '').trim();
  const tokens = ['%20', ' ', '"', "'", 'a', '\t', '%2', '<'];
  let checked = 0;
  const walk = (prefix, depth) => {
    const input = prefix.trim();      // the parser trims the destination first
    assert.equal(stripLinkTitle(input), oracle(input), JSON.stringify(input));
    checked++;
    if (depth === 0) return;
    for (const token of tokens) walk(prefix + token, depth - 1);
  };
  walk('', 6);
  assert.ok(checked > 200000);
  for (const title of ['page.png "Caption"', "page.png 'Caption'", 'page.png%20"Caption"', 'a "b" "c"', 'a"b"', 'x "it\'s"%20%20']) {
    assert.equal(stripLinkTitle(title), oracle(title), title);
  }
});

test('a link destination with a very long run of spaces is parsed in linear time', () => {
  for (const spaces of [5000, 40000]) {
    const started = performance.now();
    parser.render('[a](x' + ' '.repeat(spaces) + 'y)');
    assert.ok(performance.now() - started < 300, `${spaces} spaces took ${Math.round(performance.now() - started)} ms`);
  }
  // Titles still work next to long whitespace.
  assert.match(parser.render('![a](images/photo one.png "Caption")'), /src="images\/photo%20one.png"/);
});

test('a line with thousands of unclosed link starts is parsed in linear time', () => {
  // Each start used to read to the end of the line looking for its `)`: about 1 s at 4000 starts.
  for (const unit of ['![a](x.png "a ', '![a](x.png (t ', '[a](x.png "a ', '![a]( ']) {
    for (const closer of ['', ')']) {
      const started = performance.now();
      parser.render(unit.repeat(8000) + closer);
      assert.ok(performance.now() - started < 400, `${JSON.stringify(unit)} x 8000${closer} took ${Math.round(performance.now() - started)} ms`);
    }
  }
  // Legacy links still resolve after many failed starts on the same line, and on later lines.
  const noise = '[a]( '.repeat(500);
  assert.match(parser.render(noise + '[doc](my file.md) and ![pic](photo one.png "Caption")'), /href="my%20file.md".*src="photo%20one.png"/s);
  assert.match(parser.render(noise + '\n[doc](my file.md)'), /href="my%20file.md"/);
});

test('angle-bracket text that is not HTML stays text', () => {
  assert.equal(parser.render('returns List<String> or Map<K,V> here'), '<p>returns List&lt;String&gt; or Map&lt;K,V&gt; here</p>\n');
  assert.equal(parser.render('Run `x` as <username> then <my-tag a="1">'), '<p data-helix-code="">Run <code style="white-space: pre-wrap">x</code> as &lt;username&gt; then &lt;my-tag a=&quot;1&quot;&gt;</p>\n');
  // A placeholder alone on a line would otherwise be an HTML block.
  assert.equal(parser.render('<GroupName>\n\nafter'), '<p>&lt;GroupName&gt;</p>\n<p>after</p>\n');
  assert.equal(parser.render('<Foo>\ntext  \n</Foo>'), '<p>&lt;Foo&gt;<br>text<br>&lt;/Foo&gt;</p>\n');
});

test('real HTML elements are still HTML', () => {
  assert.equal(parser.render('a <b>bold</b> and <br> and <kbd>K</kbd>'), '<p>a <b>bold</b> and <br> and <kbd>K</kbd></p>\n');
  assert.equal(parser.render('<div class="x">there</div>'), '<div class="x">there</div>');
  assert.equal(parser.render('<details><summary>S</summary>\n\nbody\n\n</details>'), '<details><summary>S</summary>\n<p>body</p>\n</details>');
});

test('HTML comments are kept as text, the blank-line marker is not', () => {
  assert.equal(parser.render('a <!-- note to self --> b'), '<p>a &lt;!-- note to self --&gt; b</p>\n');
  assert.equal(parser.render('before\n\n<!--\nline one\nline two\n-->\n\nafter'), '<p>before</p>\n<p>&lt;!--<br>line one<br>line two<br>--&gt;</p>\n<p>after</p>\n');
  assert.equal(parser.render('one\n\n<!-- -->\n\ntwo'), '<p>one</p>\n<p></p>\n<p>two</p>\n');
  assert.equal(parser.render('x <!-- --> y'), '<p>x <!-- --> y</p>\n');
});
