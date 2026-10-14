import MarkdownIt from 'markdown-it';
import type Token from 'markdown-it/lib/token.mjs';
import type StateBlock from 'markdown-it/lib/rules_block/state_block.mjs';
import type StateInline from 'markdown-it/lib/rules_inline/state_inline.mjs';
import fenceRule from 'markdown-it/lib/rules_block/fence.mjs';
import markdownItMark from 'markdown-it-mark';
import markdownItSup from 'markdown-it-sup';
import markdownItSub from 'markdown-it-sub';
import { IMG_GRID_CLOSER, parseImgGridHeader, splitTrailingCloser } from './imageGrid.ts';

interface WikiTitle { path: string; title: string }
export interface MarkdownEnvironment { wikiLinkTitles?: readonly WikiTitle[] }
type WikiIndex = { paths: Map<string, string>; titles: Map<string, { path: string; depth: number }> };
// One index per titles array: callers replace the array, never mutate it, when a title changes.
const wikiIndexes = new WeakMap<readonly WikiTitle[], WikiIndex>();

function displayMath(state: StateBlock, startLine: number, endLine: number, silent: boolean): boolean {
  const start = state.bMarks[startLine] + state.tShift[startLine];
  if (state.src.slice(start, state.eMarks[startLine]).trim() !== '$$') return false;
  let closing = startLine + 1;
  for (; closing < endLine; closing++) {
    if (state.sCount[closing] < state.blkIndent && !state.isEmpty(closing)) return false;
    const from = state.bMarks[closing] + state.tShift[closing];
    if (state.src.slice(from, state.eMarks[closing]).trim() === '$$') break;
  }
  if (closing === endLine) return false;
  if (silent) return true;
  const token = state.push('helix_math_block', 'div', 0);
  token.block = true;
  token.content = state.getLines(startLine + 1, closing, state.blkIndent, false).trim();
  token.map = [startLine, closing + 1];
  token.meta = { sourceStart: start };
  token.attrSet('data-math-block', encodeURIComponent(token.content));
  token.attrSet('class', 'math-block');
  state.line = closing + 1;
  return true;
}

function inlineMath(state: StateInline, silent: boolean): boolean {
  const start = state.pos;
  if (state.src[start] !== '$' || state.src[start - 1] === '$' ||
    start + 1 >= state.posMax || /[\s$]/u.test(state.src[start + 1])) return false;
  let end = start + 1;
  for (; end < state.posMax; end++) {
    const char = state.src[end];
    if (char === '\n') return false;
    if (char === '\\') { end++; continue; }
    if (char === '$') break;
  }
  if (end >= state.posMax || end === start + 1 || /\s/u.test(state.src[end - 1]) ||
    state.src[end + 1] === '$' || /[0-9]/.test(state.src[end + 1] ?? '')) return false;
  if (!silent) {
    const token = state.push('helix_math_inline', 'span', 0);
    token.content = state.src.slice(start + 1, end).trim();
    token.meta = { from: start, to: end + 1 };
    token.attrSet('data-math-inline', encodeURIComponent(token.content));
    token.attrSet('class', 'math-inline');
  }
  state.pos = end + 1;
  return true;
}

// markdown-it trims two spaces even when a code span contains only spaces.
// Preserve CommonMark's all-space exception without replacing its normal backtick parser.
function spaceCode(state: StateInline, silent: boolean): boolean {
  if (state.src[state.pos] !== '`') return false;
  let content = state.pos;
  while (content < state.posMax && state.src[content] === '`') content++;
  const length = content - state.pos;
  let closing = content;
  while (closing < state.posMax && (state.src[closing] === ' ' || state.src[closing] === '\n')) closing++;
  if (closing === content) return false;
  let end = closing;
  while (end < state.posMax && state.src[end] === '`') end++;
  if (end - closing !== length) return false;
  if (!silent) {
    const token = state.push('code_inline', 'code', 0);
    token.markup = state.src.slice(state.pos, content);
    token.content = state.src.slice(content, closing).replace(/\n/g, ' ');
  }
  state.pos = end;
  return true;
}

function installMath(md: MarkdownIt) {
  md.block.ruler.before('fence', 'helix_math_block', displayMath, { alt: ['paragraph', 'reference', 'blockquote', 'list'] });
  md.inline.ruler.after('backticks', 'helix_math_inline', inlineMath);
  md.renderer.rules.helix_math_block = (tokens, index, _options, _env, renderer) =>
    `<div${renderer.renderAttrs(tokens[index])}></div>\n`;
  md.renderer.rules.helix_math_inline = (tokens, index, _options, _env, renderer) =>
    `<span${renderer.renderAttrs(tokens[index])}></span>`;
}

function wikiLink(state: StateInline, silent: boolean): boolean {
  const notes: readonly WikiTitle[] | undefined = state.env.wikiLinkTitles;
  if (!notes || state.src.slice(state.pos, state.pos + 2) !== '[[') return false;
  const end = state.src.indexOf(']]', state.pos + 2);
  if (end < 0 || end + 2 > state.posMax) return false;
  const raw = state.src.slice(state.pos + 2, end);
  if (!raw || raw.includes(']')) return false;
  if (!silent) {
    let index = wikiIndexes.get(notes);
    if (!index) {
      index = { paths: new Map(), titles: new Map() };
      for (const note of notes) {
        if (!index.paths.has(note.path)) index.paths.set(note.path, note.path);
        const title = note.title.toLowerCase();
        let depth = 0;
        for (let i = 0; i < note.path.length; i++) if (note.path[i] === '/') depth++;
        const previous = index.titles.get(title);
        if (!previous || depth < previous.depth) index.titles.set(title, { path: note.path, depth });
      }
      wikiIndexes.set(notes, index);
    }
    const pipe = raw.indexOf('|');
    const reference = (pipe < 0 ? raw : raw.slice(0, pipe)).trim();
    const display = (pipe < 0 ? reference : raw.slice(pipe + 1)).trim();
    const title = reference.replace(/#.*$/, '').replace(/\^.*$/, '').trim();
    const basename = title.includes('/') ? title.slice(title.lastIndexOf('/') + 1) : title;
    const path = (title.includes('/') ? index.paths.get(title + '.md') : undefined)
      ?? index.titles.get(basename.toLowerCase())?.path ?? '';
    const token = state.push('helix_wiki', 'span', 0);
    token.attrSet('data-wiki-link', '');
    token.attrSet('data-path', path);
    token.attrSet('data-title', reference);
    token.attrSet('class', 'wiki-link');
    token.content = display;
  }
  state.pos = end + 2;
  return true;
}

/**
 * Remove a trailing `"title"` or `'title'` (preceded by whitespace or %20, optionally followed by %20) from a
 * legacy link destination. A single scan from the end: the equivalent regular expression is quadratic on a
 * long run of spaces.
 */
export function stripLinkTitle(destination: string): string {
  const skippableBefore = (index: number) => (index >= 3 && destination.startsWith('%20', index - 3) ? 3 : /\s/u.test(destination[index - 1] ?? '') ? 1 : 0);
  let end = destination.length;
  for (let width = skippableBefore(end); width; width = skippableBefore(end)) end -= width;
  const quote = destination[end - 1];
  if (quote !== '"' && quote !== "'") return destination.trim();
  const open = destination.lastIndexOf(quote, end - 2);
  if (open <= 0) return destination.trim();
  let start = open;
  for (let width = skippableBefore(start); width; width = skippableBefore(start)) start -= width;
  return start === open ? destination.trim() : destination.slice(0, start).trim();
}

/** The title text after a legacy destination (`"title"` or `'title'`, maybe after `>` or %20), or null. */
function legacyImageTitle(rest: string): string | null {
  let text = rest.startsWith('>') ? rest.slice(1) : rest;
  text = text.replace(/^(?:\s|%20)+/u, '');
  while (text.endsWith('%20') || /\s$/u.test(text)) text = text.endsWith('%20') ? text.slice(0, -3) : text.slice(0, -1);
  const quote = text[0];
  if ((quote !== '"' && quote !== "'") || text.length < 2 || text[text.length - 1] !== quote) return null;
  return text.slice(1, -1);
}

/**
 * Whether `src[start, end)` is a valid CommonMark destination with an optional title. Such images
 * are left to markdown-it's own image rule, which handles escaped quotes and `(title)` correctly.
 */
function isCommonMarkDestination(state: StateInline, start: number, end: number): boolean {
  const { helpers } = state.md;
  const skipSpace = (pos: number) => {
    while (pos < end && /[ \t\n]/u.test(state.src[pos])) pos++;
    return pos;
  };
  let pos = skipSpace(start);
  const destination = helpers.parseLinkDestination(state.src, pos, end);
  if (!destination.ok) return false;
  pos = destination.pos;
  const afterDestination = pos;
  pos = skipSpace(pos);
  if (pos < end && pos !== afterDestination) {
    const title = helpers.parseLinkTitle(state.src, pos, end);
    if (!title.ok) return false;
    pos = skipSpace(title.pos);
  }
  return pos === end;
}

/**
 * Looking for the `)` that ends a destination reads to the end of the line when there is none, and a
 * line can hold thousands of `[label](` starts (pasted or generated text), which made parsing it
 * quadratic. Two guards keep it linear: no search starts beyond the block's last `)`, and once the
 * searches that found nothing have read this many characters in one block, the rest of the block
 * is left to the CommonMark link rule.
 */
const LEGACY_LINK_SCAN_BUDGET = 2_000_000;
const legacyLinkScans = new WeakMap<StateInline, { max: number; lastClose: number; wasted: number }>();

/** Preserve legacy space-containing and %20/title destinations only in real link syntax. */
function legacyLink(state: StateInline, silent: boolean): boolean {
  const start = state.pos;
  const image = state.src[start] === '!' && state.src[start + 1] === '[';
  const labelStart = start + (image ? 1 : 0);
  if (state.src[labelStart] !== '[') return false;
  const labelEnd = state.md.helpers.parseLinkLabel(state, labelStart, !image);
  if (labelEnd < 0 || state.src[labelEnd + 1] !== '(') return false;
  const destinationStart = labelEnd + 2;
  let scans = legacyLinkScans.get(state);
  if (!scans || scans.max < state.posMax) {
    scans = { max: state.posMax, lastClose: state.src.lastIndexOf(')', state.posMax - 1), wasted: scans?.wasted ?? 0 };
    legacyLinkScans.set(state, scans);
  }
  if (destinationStart > scans.lastClose || scans.wasted > LEGACY_LINK_SCAN_BUDGET) return false;
  let end = destinationStart;
  let depth = 0;
  let quote = '';
  let angle = false;
  for (; end < state.posMax; end++) {
    const char = state.src[end];
    if (char === '\n') { scans.wasted += end - destinationStart; return false; }
    if (char === '\\') { end++; continue; }
    if (quote) { if (char === quote) quote = ''; continue; }
    if (angle) { if (char === '>') angle = false; continue; }
    if (char === '<') { angle = true; continue; }
    if ((char === '"' || char === "'") &&
      (/\s/u.test(state.src[end - 1]) || state.src.slice(end - 3, end) === '%20')) { quote = char; continue; }
    if (char === '(') depth++;
    else if (char === ')') { if (!depth) break; depth--; }
  }
  if (end >= state.posMax) { scans.wasted += end - destinationStart; return false; }
  const raw = state.src.slice(destinationStart, end);
  // Links and images with a valid CommonMark destination and title keep the native parser (and
  // their title). A `%20"title"` suffix (exported by some apps) is still legacy syntax, not part of the path.
  if (!/%20["'(]/u.test(raw) && isCommonMarkDestination(state, destinationStart, end)) return false;
  // Native CommonMark links, including nested destinations, retain their normal parser.
  if (!/\s|%20["'(]/u.test(raw)) return false;
  let destination = raw.trim();
  if (destination.startsWith('<')) {
    const angle = destination.match(/^<([^>]*)>(?:(?:\s|%20)*(?:"[^"]*"|'[^']*'|\([^)]*\)))?(?:\s|%20)*$/u);
    if (!angle) return false;
    destination = angle[1];
  } else {
    destination = stripLinkTitle(destination);
  }
  const href = state.md.normalizeLink(destination.replace(/ /g, '%20'));
  if (!state.md.validateLink(href)) return false;
  if (!silent) {
    const label = state.src.slice(labelStart + 1, labelEnd);
    const from = destinationStart + raw.indexOf(destination);
    const sourceHref = { from, to: from + destination.length };
    if (image) {
      const children: Token[] = [];
      state.md.inline.parse(label, state.md, state.env, children);
      const token = state.push('image', 'img', 0);
      token.meta = { sourceHref };
      token.attrSet('src', href.replace(/^\/{2,}(?=home\/)/u, '/'));
      token.attrSet('alt', '');
      const title = legacyImageTitle(raw.slice(raw.indexOf(destination) + destination.length));
      if (title) token.attrSet('title', state.md.utils.unescapeAll(title));
      token.children = children;
      token.content = label;
    } else {
      const token = state.push('link_open', 'a', 1);
      token.meta = { sourceHref };
      token.attrSet('href', href);
      const title = legacyImageTitle(raw.slice(raw.indexOf(destination) + destination.length));
      if (title) token.attrSet('title', state.md.utils.unescapeAll(title));
      const oldMax = state.posMax;
      state.pos = labelStart + 1;
      state.posMax = labelEnd;
      const linkState = state as StateInline & { linkLevel: number };
      linkState.linkLevel++;
      state.md.inline.tokenize(state);
      linkState.linkLevel--;
      state.push('link_close', 'a', -1);
      state.posMax = oldMax;
    }
  }
  state.pos = end + 1;
  return true;
}

const EMPTY_PARAGRAPH_MARKER = '<!-- -->';

function emptyParagraph(state: StateBlock, startLine: number, endLine: number, silent: boolean): boolean {
  const from = state.bMarks[startLine] + state.tShift[startLine];
  const line = state.src.slice(from, state.eMarks[startLine]);
  // Versions before 1.3.7 glued the marker to a following image (`<!-- -->![](a.png)`), which
  // reads as a raw HTML line and showed the image as text. Read it as the empty paragraph and the
  // image it was written from.
  const glued = line.startsWith(EMPTY_PARAGRAPH_MARKER) && line.slice(EMPTY_PARAGRAPH_MARKER.length).trimStart().startsWith('![');
  if (!glued && line.trim() !== EMPTY_PARAGRAPH_MARKER) return false;
  if (silent) return true;
  const token = state.push('helix_empty', 'p', 0);
  token.block = true;
  token.map = [startLine, startLine + 1];
  if (!glued) {
    state.line = startLine + 1;
    return true;
  }
  const rest = from + EMPTY_PARAGRAPH_MARKER.length;
  state.bMarks[startLine] = rest;
  state.tShift[startLine] = state.src.slice(rest, state.eMarks[startLine]).search(/\S/);
  state.md.block.tokenize(state, startLine, endLine);
  return true;
}

/** True when `text` is exactly one Markdown image and nothing else (no caption, no link around it). */
function isSingleImage(state: StateBlock, text: string): boolean {
  if (!text.startsWith('![')) return false;
  const tokens: Token[] = [];
  state.md.inline.parse(text, state.md, state.env, tokens);
  return tokens.length === 1 && tokens[0].type === 'image';
}

/**
 * `:::img-grid` image grids (see imageGrid.ts). As a block rule, it never sees lines inside code
 * blocks. A block that isn't a well-formed grid is left completely to the other rules.
 */
function imageGrid(state: StateBlock, startLine: number, endLine: number, silent: boolean): boolean {
  if (state.sCount[startLine] - state.blkIndent >= 4) return false;
  const lineText = (line: number) => state.src.slice(state.bMarks[line] + state.tShift[line], state.eMarks[line]);
  const header = lineText(startLine);
  if (!header.startsWith(':::img-grid')) return false;
  const attrs = parseImgGridHeader(header);
  if (!attrs) return false;
  const images: { line: number; content: string }[] = [];
  let closing = -1;
  for (let line = startLine + 1; line < endLine && closing < 0; line++) {
    if (state.isEmpty(line) || state.sCount[line] < state.blkIndent) return false;
    const text = lineText(line).replace(/[ \t]+$/, '');
    if (text === IMG_GRID_CLOSER) {
      closing = line;
      break;
    }
    const beforeCloser = splitTrailingCloser(text);
    const image = beforeCloser ?? text;
    if (!isSingleImage(state, image)) return false;
    images.push({ line, content: image });
    if (beforeCloser !== null) closing = line;
  }
  if (closing < 0 || images.length === 0) return false;
  if (silent) return true;
  const open = state.push('helix_img_grid_open', 'div', 1);
  open.block = true;
  open.map = [startLine, closing + 1];
  open.attrSet('data-img-grid', '');
  open.attrSet('data-cols', String(attrs.cols));
  open.attrSet('data-layout', attrs.layout);
  open.attrSet('class', 'img-grid');
  for (const image of images) {
    const inline = state.push('inline', '', 0);
    inline.content = image.content;
    inline.map = [image.line, image.line + 1];
    inline.children = [];
  }
  state.push('helix_img_grid_close', 'div', -1).block = true;
  state.line = closing + 1;
  return true;
}

// Elements a note may hold as HTML. Anything else written in angle brackets is the user's text,
// such as `List<String>`, and a browser would drop it without a trace.
const HTML_ELEMENTS = new Set((
  'a abbr address area article aside audio b base bdi bdo big blockquote body br button canvas caption center cite code '
  + 'col colgroup data datalist dd del details dfn dialog dir div dl dt em embed fieldset figcaption figure font footer '
  + 'form frame frameset h1 h2 h3 h4 h5 h6 head header hgroup hr html i iframe img input ins kbd label legend li link '
  + 'main map mark menu meta meter nav nobr noscript object ol optgroup option output p param picture pre progress q rp '
  + 'rt ruby s samp script search section select slot small source span strike strong style sub summary sup svg math '
  + 'table tbody td template textarea tfoot th thead time title tr track tt u ul var video wbr'
).split(' '));
const HTML_PIECE = /<!--[\s\S]*?-->|<\/?([A-Za-z][A-Za-z0-9-]*)(?:\s[^<>]*)?\/?>/g;

/** Whether a tag or comment is shown as the text it is instead of being handed to the browser. */
function isLiteralHtml(piece: string, tagName: string | undefined): boolean {
  // A comment would vanish on the first save; the empty one is the editor's own blank-line marker.
  if (piece.startsWith('<!--')) return piece.replace(/\s/g, '') !== '<!---->';
  return !!tagName && !HTML_ELEMENTS.has(tagName.toLowerCase());
}

function literalHtml(state: { tokens: Token[]; md: MarkdownIt }): void {
  for (const token of state.tokens) {
    if (token.type === 'inline') {
      for (const child of token.children ?? []) {
        if (child.type !== 'html_inline') continue;
        HTML_PIECE.lastIndex = 0;
        const match = HTML_PIECE.exec(child.content);
        if (match && match[0] === child.content && isLiteralHtml(match[0], match[1])) child.type = 'text';
      }
    } else if (token.type === 'html_block') {
      let literal = false;
      const content = token.content.replace(HTML_PIECE, (piece, tagName) => {
        if (!isLiteralHtml(piece, tagName)) return piece;
        literal = true;
        return state.md.utils.escapeHtml(piece);
      });
      if (!literal) continue;
      // Text without an element around it becomes a paragraph that keeps its lines.
      token.content = content.startsWith('&lt;')
        ? `<p>${content.trim().split('\n').map((line) => line.trimEnd()).join('<br>')}</p>\n` : content;
    }
  }
}

export function createEditorMarkdownParser(): MarkdownIt {
  const md = MarkdownIt({ html: true, linkify: false, breaks: false })
    .use(markdownItMark).use(markdownItSup).use(markdownItSub);
  // Existing editor convention: tab-indented prose does not become an indented code block.
  md.disable('code');
  installMath(md);
  md.inline.ruler.before('link', 'helix_wiki', wikiLink);
  md.inline.ruler.before('backticks', 'helix_space_code', spaceCode);
  md.inline.ruler.before('link', 'helix_legacy_link', legacyLink);
  md.block.ruler.before('html_block', 'helix_empty', emptyParagraph, { alt: ['paragraph', 'reference', 'blockquote', 'list'] });
  md.block.ruler.before('fence', 'helix_img_grid', imageGrid, { alt: ['paragraph', 'reference', 'blockquote', 'list'] });
  md.renderer.rules.helix_empty = () => '<p></p>\n';
  md.renderer.rules.helix_wiki = (tokens, index, _options, _env, renderer) =>
    `<span${renderer.renderAttrs(tokens[index])}>${md.utils.escapeHtml(tokens[index].content)}</span>`;

  md.core.ruler.after('inline', 'helix_literal_html', literalHtml);

  md.core.ruler.before('inline', 'helix_tasks', state => {
    const lists: { open: Token; firstItem: boolean; item: Token | null; firstParagraph: boolean }[] = [];
    for (const token of state.tokens) {
      if (token.type === 'bullet_list_open' || token.type === 'ordered_list_open') {
        lists.push({ open: token, firstItem: true, item: null, firstParagraph: true });
      } else if (token.type === 'bullet_list_close' || token.type === 'ordered_list_close') lists.pop();
      else if (token.type === 'list_item_open' && lists.length) {
        const list = lists[lists.length - 1];
        list.item = token;
        list.firstParagraph = true;
      } else if (token.type === 'list_item_close' && lists.length) {
        const list = lists[lists.length - 1];
        list.item = null;
        list.firstItem = false;
      } else if (token.type === 'inline' && lists.length) {
        const list = lists[lists.length - 1];
        if (!list.item || !list.firstParagraph || token.level !== list.item.level + 2) continue;
        list.firstParagraph = false;
        // Task items only exist in bullet lists; `1. [x] text` stays an ordinary numbered item.
        if (list.open.type !== 'bullet_list_open') continue;
        const checkbox = token.content.match(/^\[(x| )\](?:[^\S\n]+|$)/u);
        if (!checkbox) continue;
        token.content = token.content.slice(checkbox[0].length);
        list.item.attrSet('data-type', 'taskItem');
        list.item.attrSet('data-checked', checkbox[1] === 'x' ? 'true' : 'false');
        if (list.firstItem) list.open.attrSet('data-type', 'taskList');
      }
    }
  });

  md.core.ruler.after('inline', 'helix_images', state => {
    let paragraph: Token | null = null;
    let cell: Token | null = null;
    // Tight list items render no <p>, so a simple item (open, hidden paragraph, inline, close) carries the marker.
    const items: { token: Token; index: number; hasCode: boolean }[] = [];
    let lines: string[] | undefined;
    for (const [index, token] of state.tokens.entries()) {
      if (token.type === 'paragraph_open' || token.type === 'heading_open') paragraph = token;
      else if (token.type === 'paragraph_close' || token.type === 'heading_close') paragraph = null;
      else if (token.type === 'th_open' || token.type === 'td_open') cell = token;
      else if (token.type === 'th_close' || token.type === 'td_close') cell = null;
      else if (token.type === 'list_item_open') items.push({ token, index, hasCode: false });
      else if (token.type === 'list_item_close') {
        const item = items.pop();
        if (item?.hasCode && index - item.index === 4) item.token.attrSet('data-helix-code', '');
      }
      if (token.type !== 'inline') continue;
      for (const child of token.children ?? []) {
        // ProseMirror's DOM parser otherwise collapses or drops whitespace-only code spans.
        if (child.type === 'code_inline') {
          child.attrSet('style', 'white-space: pre-wrap');
          paragraph?.attrSet('data-helix-code', '');
          cell?.attrSet('data-helix-code', '');
          if (paragraph?.hidden && items.length) items[items.length - 1].hasCode = true;
        }
        if (child.type === 'image') {
          const src = child.attrGet('src');
          if (src?.startsWith('//')) child.attrSet('src', src.replace(/^\/{2,}(?=home\/)/u, '/'));
        }
      }
      if (!paragraph || paragraph.type !== 'paragraph_open' || paragraph.level !== 0 || token.children?.length !== 1 ||
        token.children[0].type !== 'image' || !token.map || token.map[0] < 2) continue;
      lines ??= state.src.split('\n');
      const line = token.map[0];
      if (lines[line].startsWith('![') && lines[line - 1].trim() === '') paragraph.meta = { helixImageGap: true };
    }
  });
  // Callout rendering splits the title line from the body at the first newline of the opening
  // paragraph, so that one soft break must stay a newline instead of becoming a space.
  md.core.ruler.after('inline', 'helix_callout_title', state => {
    for (let index = 0; index + 2 < state.tokens.length; index++) {
      const [quote, paragraph, inline] = [state.tokens[index], state.tokens[index + 1], state.tokens[index + 2]];
      if (quote.type !== 'blockquote_open' || paragraph.type !== 'paragraph_open' || inline.type !== 'inline') continue;
      if (!/^\[![\w-]+\]/u.test(inline.content)) continue;
      const softbreak = inline.children?.find(child => child.type === 'softbreak');
      if (softbreak) softbreak.meta = { helixCalloutTitle: true };
    }
  });
  md.renderer.rules.paragraph_open = (tokens, index, options, _env, renderer) =>
    (tokens[index].meta?.helixImageGap ? '<p></p>\n' : '') + renderer.renderToken(tokens, index, options);

  md.block.ruler.at('fence', (state, startLine, endLine, silent) => {
    if (!fenceRule(state, startLine, endLine, silent)) return false;
    if (silent) return true;
    const token = state.tokens[state.tokens.length - 1];
    if (token.info.trim() === 'helix-secret' && token.map) {
      const line = token.map[1] - 1;
      const closing = state.src.slice(state.bMarks[line] + state.tShift[line], state.eMarks[line]);
      let length = 0;
      while (closing[length] === token.markup[0]) length++;
      const indentation = state.sCount[line] - state.blkIndent;
      token.meta = { helixSecretClosed: indentation >= 0 && indentation < 4 &&
        length >= token.markup.length && /^[ \t]*$/u.test(closing.slice(length)) };
    }
    return true;
  }, { alt: ['paragraph', 'reference', 'blockquote', 'list'] });
  const renderFence = md.renderer.rules.fence!;
  md.renderer.rules.fence = (tokens, index, options, env, renderer) => {
    const token = tokens[index];
    if (token.info.trim() === 'helix-secret' && token.meta?.helixSecretClosed) {
      return `<div data-secret-block="${md.utils.escapeHtml(encodeURIComponent(token.content.replace(/\n$/, '')))}"></div>\n`;
    }
    // Only the final structural LF is parser-added; leading and extra trailing blank lines are literal.
    return renderFence(tokens, index, options, env, renderer).replace(/\n<\/code>/u, '</code>');
  };
  md.renderer.rules.softbreak = (tokens, index) => (tokens[index].meta?.helixCalloutTitle ? '\n' : ' ');
  md.renderer.rules.hardbreak = () => '<br>';
  return md;
}

const mathBlocks = MarkdownIt({ html: true });
installMath(mathBlocks);
mathBlocks.inline.ruler.before('link', 'helix_legacy_link', legacyLink);

/**
 * Inline tokens of each blank-line-separated block, parsed on its own like the editor does, with
 * source ranges relative to the whole note. Inline syntax such as a code span cannot pair across
 * blocks, so one stray backtick must not hide the links and formulas after it.
 */
function inlineTokensByBlock(source: string): Token[] {
  const tokens: Token[] = [];
  let start = 0;
  let offset = 0;
  const flush = (end: number) => {
    if (end <= start) return;
    const found: Token[] = [];
    mathBlocks.inline.parse(source.slice(start, end), mathBlocks, {}, found);
    for (const token of found) {
      if (token.meta?.sourceHref) {
        const { from, to } = token.meta.sourceHref;
        token.meta = { ...token.meta, sourceHref: { from: from + start, to: to + start } };
      } else if (token.type === 'helix_math_inline') {
        token.meta = { ...token.meta, from: token.meta.from + start, to: token.meta.to + start };
      }
      tokens.push(token);
    }
  };
  for (const line of source.split('\n')) {
    if (/^[ \t\r]*$/.test(line)) {
      flush(offset);
      start = offset + line.length + 1;
    }
    offset += line.length + 1;
  }
  flush(source.length);
  return tokens;
}

/** Source destinations for the editor's legacy space/title link compatibility. */
export function legacyLinkRanges(source: string): { from: number; to: number }[] {
  if (!source.includes('](')) return [];
  return inlineTokensByBlock(source).filter(token => token.meta?.sourceHref).map(token => token.meta.sourceHref);
}

/** Inline math coordinates use the same delimiter and escape rules as rendering. */
export function inlineMathRanges(source: string): { from: number; to: number }[] {
  if (!source.includes('$')) return [];
  return inlineTokensByBlock(source).filter(token => token.type === 'helix_math_inline').map(token => token.meta);
}

/** Source coordinates of semantic display-math blocks, including nested containers. */
export function displayMathRanges(source: string): { from: number; to: number; prefix: string }[] {
  if (!source.includes('$$')) return [];
  const tokens: Token[] = [];
  mathBlocks.block.parse(source, mathBlocks, {}, tokens);
  const ranges = tokens.filter(token => token.type === 'helix_math_block' && token.map);
  if (!ranges.length) return [];
  const starts = [0];
  for (let i = 0; i < source.length; i++) if (source[i] === '\n') starts.push(i + 1);
  return ranges.map(token => {
    const from = starts[token.map![0]];
    return { from, to: starts[token.map![1]] ?? source.length,
      prefix: source.slice(from, token.meta.sourceStart) };
  });
}
