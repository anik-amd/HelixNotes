/*
 * Pasting Markdown into the rich editor. Plain text that uses Markdown syntax is rendered the way a
 * note is; anything else keeps the normal paste. Clipboard HTML that is only styled text (a code
 * editor or terminal copies its colors as HTML) does not count as formatting.
 */

const BLOCK_SYNTAX = /^(?: {0,3}#{1,6}[ \t]+\S| {0,3}>|[ \t]*(?:[-*+]|\d+[.)])[ \t]+\S| {0,3}(?:```|~~~)|[ \t]*\|.*\|[ \t]*$| {0,3}(?:-{3,}|\*{3,}|_{3,})[ \t]*$|:::img-grid\b)/m;
const INLINE_SYNTAX = /\*\*[^*\n]+\*\*|__[^_\n]+__|~~[^~\n]+~~|==[^=\n]+==|`[^`\n]+`|!?\[[^\]\n]*\]\([^)\s]+(?:\s+"[^"\n]*")?\)|\[\[[^\]\n]+\]\]|(?:^|[\s(])\*[^*\s][^*\n]*\*(?=[\s).,;:!?]|$)/m;

/** Whether pasted plain text is written in Markdown rather than meant literally. */
export function looksLikeMarkdown(text: string): boolean {
	if (!text.trim()) return false;
	// A lone address is a link paste, handled on its own.
	if (/^\s*[a-z][a-z0-9+.-]*:\/\/\S+\s*$/i.test(text)) return false;
	return BLOCK_SYNTAX.test(text) || INLINE_SYNTAX.test(text);
}

/** Whether clipboard HTML carries structure of its own (headings, lists, links, emphasis...). */
export function htmlHasFormatting(html: string): boolean {
	return /<(?:h[1-6]|ul|ol|li|table|blockquote|pre|code|a|img|strong|b|em|i|u|s|del|mark|hr)\b/i.test(html);
}
