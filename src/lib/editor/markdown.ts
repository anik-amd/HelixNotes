import type { Mark, Node as ProseMirrorNode } from '@tiptap/pm/model';

type AssetSourceNormalizer = (source: string) => string;

type MarkdownMark = {
  key: string;
  priority: number;
  open: string;
  close: string;
};

const identityAssetSource: AssetSourceNormalizer = (source) => source;

function backtickDelimiter(text: string, minimum: number): string {
  let length = minimum;
  for (const match of text.matchAll(/`+/g)) {
    length = Math.max(length, match[0].length + 1);
  }
  return '`'.repeat(length);
}

export interface MarkdownBlock {
  text: string;
  isImage: boolean;
  /** An empty paragraph, which is kept as a marker line so its vertical space round-trips. */
  isEmpty?: boolean;
}

const EMPTY_PARAGRAPH_MARKER = '<!-- -->';

/**
 * ` "title"` suffix for an image destination, or '' without a title. Uses CommonMark backslash
 * escapes, which the parser undoes exactly, so a title survives any number of saves unchanged.
 * `&` is only escaped where it would otherwise read as an entity (`&amp;`, `&#38;`).
 */
export function serializeImageTitle(title: string | null | undefined): string {
  if (!title) return '';
  return ` "${title.replace(/[\\"]/g, '\\$&').replace(/&(?=#?[a-z0-9]+;)/gi, '\\&')}"`;
}

/**
 * Join serialized top-level blocks into a note body. Blocks that already end in a blank line
 * (lists) get no extra separator, and an empty paragraph directly before an image is written as
 * the blank line the parser turns back into that paragraph, not as a marker glued to the image.
 */
export function joinMarkdownBlocks(blocks: readonly MarkdownBlock[]): string {
  const entries = [...blocks];
  while (entries.length > 0 && entries[entries.length - 1].isEmpty) entries.pop();
  const parts: string[] = [];
  // Only the last two characters decide the separator. Reading them from the growing result would
  // flatten it on every block, which makes joining a large note quadratic.
  let tail = '';
  const append = (piece: string) => {
    parts.push(piece);
    tail = (tail + piece).slice(-2);
  };
  entries.forEach((entry, index) => {
    if (entry.isEmpty && entries[index + 1]?.isImage) {
      if (index > 0 && tail !== '\n\n') append(tail.endsWith('\n') ? '\n' : '\n\n');
      return;
    }
    const text = entry.isEmpty ? EMPTY_PARAGRAPH_MARKER : entry.text;
    // Images follow their predecessor directly; every other block needs a blank line before it.
    if (index === 0) append(text);
    else append((entry.isImage || tail === '\n\n' ? '' : '\n') + text);
  });
  return parts.join('').trim() + '\n';
}

export interface ListItemBlock {
  text: string;
  /** A nested bullet, numbered or task list. */
  isList: boolean;
}

/**
 * Join the serialized children of one list item. The first block follows the list marker; a nested
 * list is indented four spaces directly under it; any other block (a further paragraph, a code
 * block, a quote) is separated by a blank line and indented the same way so it stays in the item.
 */
export function joinListItemBlocks(blocks: readonly ListItemBlock[]): string {
  const parts: string[] = [];
  blocks.forEach((block, index) => {
    if (block.isList) {
      // A serialized list ends with a blank line; left in, it became an indented blank line here
      // and an extra empty line after the outer list.
      parts.push(block.text.replace(/\n+$/, '').split('\n').map((line) => (line ? '    ' + line : line)).join('\n'));
    } else if (index === 0) {
      parts.push(block.text);
    } else {
      const body = block.text.replace(/\n+$/, '');
      if (body.trim() === '') return;
      parts.push('', body.split('\n').map((line) => (line ? '    ' + line : line)).join('\n'));
    }
  });
  return parts.join('\n') + '\n';
}

export function serializeCodeBlock(code: string, language: string = ''): string {
  const fence = backtickDelimiter(code, 3);
  return `${fence}${language}\n${code}\n${fence}\n`;
}

function serializeCodeSpan(text: string): string {
  const delimiter = backtickDelimiter(text, 1);
  const needsPadding = text.startsWith('`') || text.endsWith('`')
    || ((text.startsWith(' ') || text.endsWith(' ')) && /[^ ]/.test(text));
  const padding = needsPadding ? ' ' : '';
  return `${delimiter}${padding}${text}${padding}${delimiter}`;
}

function serializeInlineMath(tex: string, forceEncoded: boolean): string {
  if (forceEncoded || !tex || /[\r\n$]/.test(tex) || /^\s|\s$/.test(tex) || /(?:^|[^\\])(?:\\\\)*\\$/.test(tex)) {
    return `<span data-math-inline="${encodeURIComponent(tex)}" class="math-inline"></span>`;
  }
  return `$${tex}$`;
}

function stringAttr(mark: Mark, name: string): string {
  const value = mark.attrs?.[name];
  return typeof value === 'string' ? value : '';
}

function markdownMark(mark: Mark, text: string): MarkdownMark | null {
  switch (mark.type.name) {
    case 'bold':
      return { key: 'bold', priority: 10, open: '**', close: '**' };
    case 'italic':
      return { key: 'italic', priority: 20, open: '*', close: '*' };
    case 'strike':
      return { key: 'strike', priority: 30, open: '~~', close: '~~' };
    case 'underline':
      return { key: 'underline', priority: 40, open: '<u>', close: '</u>' };
    case 'subscript':
      return { key: 'subscript', priority: 50, open: '~', close: '~' };
    case 'superscript':
      return { key: 'superscript', priority: 60, open: '^', close: '^' };
    case 'highlight': {
      const color = stringAttr(mark, 'color');
      return color
        ? {
            key: `highlight:${color}`,
            priority: 70,
            open: `<mark data-color="${color}">`,
            close: '</mark>',
          }
        : { key: 'highlight', priority: 70, open: '==', close: '==' };
    }
    case 'textStyle': {
      const color = stringAttr(mark, 'color');
      return color
        ? {
            key: `textStyle:${color}`,
            priority: 80,
            open: `<span style="color: ${color}">`,
            close: '</span>',
          }
        : null;
    }
    case 'link': {
      const href = stringAttr(mark, 'href');
      const title = stringAttr(mark, 'title');
      return {
        key: `link:${href}:${title}`,
        priority: 100,
        open: '[',
        close: `](${href}${serializeImageTitle(title)})`,
      };
    }
    case 'wikiLink': {
      const title = stringAttr(mark, 'title') || text;
      const aliased = mark.attrs?.aliased === true || title !== text;
      return {
        key: `wikiLink:${title}:${aliased}`,
        priority: 110,
        open: aliased ? `[[${title}|` : '[[',
        close: ']]',
      };
    }
    default:
      return null;
  }
}

const escapeAttribute = (value: string) => value.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;');

/**
 * Markdown for an image. One with a width or height (from HTML such as `<img width="80">`) is
 * written as HTML again, since Markdown images have no size.
 */
export function serializeImage(node: ProseMirrorNode, source: string): string {
  const alt = node.attrs.alt || '';
  const size = node.attrs['data-size'] || node.attrs.size || 'full';
  const width = String(node.attrs.width ?? '').trim();
  const height = String(node.attrs.height ?? '').trim();
  if (width || height) {
    const attributes = [
      `src="${escapeAttribute(source)}"`,
      `alt="${escapeAttribute(alt)}"`,
      node.attrs.title ? `title="${escapeAttribute(node.attrs.title)}"` : '',
      width ? `width="${escapeAttribute(width)}"` : '',
      height ? `height="${escapeAttribute(height)}"` : '',
      size && size !== 'full' ? `data-size="${escapeAttribute(size)}"` : '',
    ].filter(Boolean);
    return `<img ${attributes.join(' ')}>`;
  }
  const sizeSuffix = size && size !== 'full' ? `|size=${size}` : '';
  return `![${alt}${sizeSuffix}](${source}${serializeImageTitle(node.attrs.title)})`;
}

function marksForText(node: ProseMirrorNode, linksOnly = false): MarkdownMark[] {
  return node.marks
    .filter((mark) => !linksOnly || mark.type.name === 'link' || mark.type.name === 'wikiLink')
    .map((mark) => markdownMark(mark, node.text || ''))
    .filter((mark): mark is MarkdownMark => mark !== null)
    .sort((left, right) => left.priority - right.priority || left.key.localeCompare(right.key));
}

function commonMarkCount(active: MarkdownMark[], next: MarkdownMark[]): number {
  const length = Math.min(active.length, next.length);
  let index = 0;
  while (index < length && active[index].key === next[index].key) index += 1;
  return index;
}

export interface InlineSerializeOptions {
  /** Leave `$` unescaped (only backslashes before it are doubled); callers must check it cannot form math. */
  literalDollars?: boolean;
  /** Rewrites the text of a plain (non-code) text child before it is written; see `literalTextEscapes`. */
  literalText?: (text: string, childIndex: number) => string;
}

export function serializeInlineMarkdown(
  node: ProseMirrorNode,
  normalizeAssetSource: AssetSourceNormalizer = identityAssetSource,
  options: InlineSerializeOptions = {},
): string {
  let result = '';
  let activeMarks: MarkdownMark[] = [];
  // Length of `result` right after each active mark was opened, to tell when a mark holds nothing.
  let openEnds: number[] = [];

  // `**word **` is not emphasis in Markdown: delimiters must touch the text they wrap. Spaces at the
  // edges of bold, italic and similar marks are therefore written outside the delimiters, and a mark
  // that covers only spaces is not written at all.
  const isDelimiter = (mark: MarkdownMark) => mark.open === mark.close && !mark.open.startsWith('<');

  const transitionMarks = (nextMarks: MarkdownMark[]) => {
    const shared = commonMarkCount(activeMarks, nextMarks);
    let trailing = '';
    if (activeMarks.slice(shared).some(isDelimiter)) {
      trailing = result.match(/[ \t\u00a0]+$/)?.[0] ?? '';
      if (trailing) result = result.slice(0, -trailing.length);
    }
    for (let index = activeMarks.length - 1; index >= shared; index -= 1) {
      const mark = activeMarks[index];
      if (isDelimiter(mark) && result.length === openEnds[index]) result = result.slice(0, result.length - mark.open.length);
      else result += mark.close;
    }
    result += trailing;
    openEnds = openEnds.slice(0, shared);
    for (let index = shared; index < nextMarks.length; index += 1) {
      result += nextMarks[index].open;
      openEnds.push(result.length);
    }
    activeMarks = nextMarks;
  };

  for (let index = 0; index < node.childCount; index += 1) {
    const child = node.child(index);
    if (child.isText) {
      const isCode = child.marks.some((mark) => mark.type.name === 'code');
      let text = child.text || '';
      if (isCode) {
        // One code span per run of code text under the same link. Formatting that covers the whole
        // run is written around the span (**`code`**); formatting on only part of it is dropped,
        // since it cannot be written inside a code span.
        const links = marksForText(child, true);
        let marks = marksForText(child);
        while (index + 1 < node.childCount) {
          const next = node.child(index + 1);
          if (!next.isText || !next.marks.some((mark) => mark.type.name === 'code')) break;
          const nextLinks = marksForText(next, true);
          if (links.length !== nextLinks.length || commonMarkCount(links, nextLinks) !== links.length) break;
          const nextKeys = new Set(marksForText(next).map((mark) => mark.key));
          marks = marks.filter((mark) => nextKeys.has(mark.key));
          text += next.text || '';
          index += 1;
        }
        // Delimiters next to a code span sit between punctuation (the backtick) and the neighbor:
        // `**\`x\`**s` is not emphasis when a letter or digit follows, nor `a**\`x\`**` when one
        // precedes. Formatting that would open or close against such a neighbor is dropped.
        const after = index + 1 < node.childCount ? node.child(index + 1) : null;
        const afterKeys = new Set(after?.isText ? marksForText(after).map((mark) => mark.key) : []);
        const wordAfter = /^[\p{L}\p{N}_]/u.test(after?.isText ? after.text || '' : '');
        const wordBefore = /[\p{L}\p{N}_]$/u.test(result);
        marks = marks.filter((mark) => !isDelimiter(mark)
          || !((wordAfter && !afterKeys.has(mark.key)) || (wordBefore && !activeMarks.some((active) => active.key === mark.key))));
        transitionMarks(marks);
        result += serializeCodeSpan(text);
      } else {
        if (options.literalText) text = options.literalText(text, index);
        if (index === 0) {
          text = text.replace(/^[\t\u2003]+/, (whitespace) => '&emsp;'.repeat(whitespace.length));
        }
        const marks = marksForText(child);
        const shared = commonMarkCount(activeMarks, marks);
        const leading = marks.slice(shared).some(isDelimiter) ? text.match(/^[ \t\u00a0]+/)?.[0] ?? '' : '';
        if (leading) {
          // Spaces before newly opened delimiters stay outside them.
          transitionMarks(marks.slice(0, shared));
          result += leading;
          text = text.slice(leading.length);
          if (!text) continue;
        }
        transitionMarks(marks);
        text = options.literalDollars
          ? text.replace(/\\+(?=\$)/g, (backslashes) => backslashes + backslashes)
          : text.replace(/\\*\$/g, (match) => '\\'.repeat((match.length - 1) * 2 + 1) + '$');
        if (index + 1 < node.childCount && node.child(index + 1).type.name === 'mathInline') {
          text = text.replace(/\\+$/, (backslashes) => backslashes + backslashes);
        }
        result += text;
      }
      continue;
    }

    if (child.type.name === 'image') {
      const source = normalizeAssetSource(child.attrs.src || '');
      if (!source) continue;
      // An image can sit inside a link (`[![logo](logo.png)](https://site)`); other formatting closes.
      transitionMarks(marksForText(child, true));
      // Images go on their own line, except right after the link around them opens.
      if (result && result.length !== openEnds[openEnds.length - 1]) result += '\n';
      result += serializeImage(child, source);
      continue;
    }
    transitionMarks([]);
    if (child.type.name === 'mathInline') {
      const previous = index > 0 ? node.child(index - 1) : null;
      const next = index + 1 < node.childCount ? node.child(index + 1) : null;
      const forceEncoded = previous?.type.name === 'mathInline' || next?.type.name === 'mathInline'
        || /\$$/.test(previous?.text || '') || /^[\d$]/.test(next?.text || '');
      result += serializeInlineMath(child.attrs.tex || '', forceEncoded);
    } else if (child.type.name === 'hardBreak') {
      result += '  \n';
    }
  }

  transitionMarks([]);
  return result;
}

// ---- Keeping plain text plain
// Text in the editor is literal: `*not italic*`, `<b>` or `&amp;` typed or loaded as text must come
// back as the same text. Written as they are, such characters can turn into formatting, HTML or an
// entity on the next load. They are escaped with a backslash, but only where the parser would
// otherwise read them differently, so ordinary notes are written exactly as before.

/**
 * Characters that can start or end inline formatting, HTML or an entity in the place they stand.
 * Cheap: no parsing. Brackets are left out on purpose: text that reads as a link or an image is
 * left to become one, as it always has (images damaged by an old save bug come back that way).
 */
const LITERAL_BACKSLASH = /\\(?=[!-#%-/:-@[-`{-~])/g;
const LITERAL_DELIMITER = /[*`~^_]|<(?=[A-Za-z/!?])|&(?=#?[A-Za-z0-9]+;)|=(?==)/g;
const LITERAL_CANDIDATE = /\\(?=[!-#%-/:-@[-`{-~])|[*`~^]|(?:^|[^\p{L}\p{N}])_|_(?![\p{L}\p{N}])|<(?=[A-Za-z/!?])|&(?=#?[A-Za-z0-9]+;)|==/u;
const MAX_MINIMIZED_ESCAPES = 40;

/** Reads a string of inline Markdown back and returns the text it shows (formatting removed). */
export type InlinePlainText = (markdown: string) => string;

const squash = (text: string) => text.replace(/[\s\u00a0\u2003]+/g, '');

/** A literal backslash before punctuation would be read as an escape, so it is always doubled. */
function escapeBackslashes(text: string): string {
  return text.replace(LITERAL_BACKSLASH, '\\$&');
}

function escapeEveryCandidate(text: string): string {
  return escapeBackslashes(text).replace(LITERAL_DELIMITER, '\\$&');
}

/** The fewest backslashes that make `text`, on its own, read back as itself. */
function escapeWhereNeeded(raw: string, plainText: InlinePlainText): string {
  const wanted = squash(raw);
  if (squash(plainText(raw)) === wanted) return raw;
  const text = escapeBackslashes(raw);
  if (squash(plainText(text)) === wanted) return text;
  const positions: number[] = [];
  for (const match of text.matchAll(LITERAL_DELIMITER)) positions.push(match.index);
  const build = (escaped: ReadonlySet<number>) => {
    let result = '';
    let from = 0;
    for (const position of positions) {
      if (!escaped.has(position)) continue;
      result += text.slice(from, position) + '\\';
      from = position;
    }
    return result + text.slice(from);
  };
  const escaped = new Set(positions);
  if (positions.length > MAX_MINIMIZED_ESCAPES) return build(escaped);
  // Last to first, so the backslashes that stay are the leading ones (`\\*not italic*`).
  for (const position of [...positions].reverse()) {
    escaped.delete(position);
    if (squash(plainText(build(escaped))) !== wanted) escaped.add(position);
  }
  return build(escaped);
}

/**
 * Decides how the plain text children of `node` must be escaped so the whole line reads back as
 * the same text, and returns that as a `literalText` option (undefined when nothing needs it).
 * Tries the unescaped form first, then each text child fixed on its own, then every candidate
 * escaped; if even that does not read back, the line is left as it was written before.
 */
export function literalTextEscapes(
  node: ProseMirrorNode,
  normalizeAssetSource: AssetSourceNormalizer,
  plainText: InlinePlainText,
): InlineSerializeOptions['literalText'] {
  // Nodes are immutable and reused between saves, so each line is judged once, not on every save.
  const known = literalDecisions.get(node);
  if (known) return known === 'none' ? undefined : known;
  const decision = decideLiteralText(node, normalizeAssetSource, plainText);
  literalDecisions.set(node, decision ?? 'none');
  return decision;
}

const literalDecisions = new WeakMap<ProseMirrorNode, NonNullable<InlineSerializeOptions['literalText']> | 'none'>();

function decideLiteralText(
  node: ProseMirrorNode,
  normalizeAssetSource: AssetSourceNormalizer,
  plainText: InlinePlainText,
): InlineSerializeOptions['literalText'] {
  let wanted = '';
  let plain = '';
  let formatted = false;
  node.forEach((child) => {
    if (!child.isText) return;
    wanted += child.text || '';
    if (child.marks.length) formatted = true;
    if (!child.marks.some((mark) => mark.type.name === 'code')) plain += (child.text || '') + '\n';
  });
  if (!LITERAL_CANDIDATE.test(plain)) return undefined;
  // Without formatting on the line, a delimiter needs a partner of its own kind to mean anything:
  // one `*`, `_`, `~`, `^` or backtick on its own stays text. This spares most lines the check.
  if (!formatted && !/\\(?=[!-#%-/:-@[-`{-~])|<(?=[A-Za-z/!?])|&(?=#?[A-Za-z0-9]+;)|==[^]*==/.test(plain)
    && !/\*[^]*\*|_[^]*_|`[^]*`|~[^]*~|\^[^]*\^/.test(plain)) return undefined;
  wanted = squash(wanted);
  const readsBack = (literalText?: InlineSerializeOptions['literalText']) =>
    squash(plainText(serializeInlineMarkdown(node, normalizeAssetSource, { literalText }))) === wanted;
  if (readsBack()) return undefined;
  const minimal = new Map<string, string>();
  const each: InlineSerializeOptions['literalText'] = (text) => {
    if (!LITERAL_CANDIDATE.test(text)) return text;
    if (!minimal.has(text)) minimal.set(text, escapeWhereNeeded(text, plainText));
    return minimal.get(text)!;
  };
  if (readsBack(each)) return each;
  const all: InlineSerializeOptions['literalText'] = (text) => escapeEveryCandidate(text);
  return readsBack(all) ? all : undefined;
}

/** Whether a serialized paragraph is still read as one paragraph. */
export type ReadsAsParagraph = (markdown: string) => boolean;

/** Whether the note, as last loaded or saved, has this line (compared without indentation). */
export type InNoteSource = (line: string) => boolean;

const BLOCK_START = /^ {0,3}(?:#{1,6}(?:\s|$)|>|[-+*](?:\s|$)|\d{1,9}[.)](?:\s|$)|`{3}|~{3}|=+\s*$|[-*_](?:\s*[-*_]){2,}\s*$|\[[^\]]*\]:|:::)/m;

/**
 * A paragraph whose line starts like a heading, quote, list item, fence, rule or link definition
 * would become that block on the next load. Such starts get a backslash, but only when the parser
 * confirms the paragraph would not come back as a paragraph.
 *
 * One exception keeps a long-standing behavior: the editor does not read indented code, so a
 * source line such as `    - item` is shown as text and has always been saved as `- item`, which
 * the next load reads as the list its author meant. A line the note's source already has in that
 * unescaped form is therefore left alone; only text from elsewhere (typed, pasted, or escaped in
 * the source) is escaped.
 */
export function escapeBlockStarts(markdown: string, readsAsParagraph: ReadsAsParagraph, inNoteSource?: InNoteSource): string {
  if (!BLOCK_START.test(markdown) || readsAsParagraph(markdown)) return markdown;
  return markdown
    .split('\n')
    .map((line) => (inNoteSource?.(line) ? line : line
      .replace(/^( {0,3})(\d{1,9})([.)])(?=\s|$)/, '$1$2\\$3')
      .replace(/^( {0,3})(#{1,6}(?:\s|$)|>|[-+*](?:\s|$)|`{3}|~{3}|=+\s*$|[-*_](?:\s*[-*_]){2,}\s*$|\[[^\]]*\]:|:::)/, '$1\\$2')))
    .join('\n');
}

/**
 * Like serializeInlineMarkdown, but writes a `$` as typed unless that could change the note's
 * math. `inlineMathPayloads` returns the TeX of each inline formula the parser finds in a string;
 * if plain dollars would add, remove or re-pair a formula, every `$` is escaped as before.
 */
export function serializeInlineMarkdownKeepingDollars(
  node: ProseMirrorNode,
  normalizeAssetSource: AssetSourceNormalizer,
  inlineMathPayloads: (markdown: string) => string[],
  plainText?: InlinePlainText,
): string {
  const literalText = plainText ? literalTextEscapes(node, normalizeAssetSource, plainText) : undefined;
  const escaped = serializeInlineMarkdown(node, normalizeAssetSource, { literalText });
  if (!escaped.includes('$')) return escaped;
  const plain = serializeInlineMarkdown(node, normalizeAssetSource, { literalDollars: true, literalText });
  // A `$$` could start a display formula in a later block, so only single dollars are left alone.
  if (plain.includes('$$')) return escaped;
  const expected = inlineMathPayloads(escaped);
  const actual = inlineMathPayloads(plain);
  return actual.length === expected.length && actual.every((tex, index) => tex === expected[index]) ? plain : escaped;
}
