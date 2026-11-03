/**
 * HTML copies are styled for the receiving document. Word, LibreOffice and many email
 * editors apply their own Normal paragraph style (typically 1.08 line spacing plus space
 * after) to bare `<p>` tags, so a copied note arrives with more vertical space than it has
 * in HelixNotes. Inline single spacing keeps the note's own blank lines as the only
 * paragraph spacing; the generated HTML stays semantic (p, h1, li, ...).
 */

const COPY_BLOCK_TAGS = 'p|h[1-6]|li|blockquote';
const COPY_BLOCK_STYLE = 'line-height:100%';
const COPY_PARAGRAPH_STYLE = 'line-height:100%;margin:0';

/**
 * Add single-line spacing to the block elements of a copied HTML fragment. Appending the
 * declarations lets them win over any inline line-height or margin already on the element.
 */
export function applySingleLineSpacing(html: string): string {
	return html.replace(
		new RegExp(`<(${COPY_BLOCK_TAGS})\\b([^>]*)>`, 'gi'),
		(_match, tag: string, attributes: string) => {
			const blockStyle = tag.toLowerCase() === 'blockquote' ? COPY_BLOCK_STYLE : COPY_PARAGRAPH_STYLE;
			if (!/\bstyle\s*=/i.test(attributes)) {
				return `<${tag}${attributes} style="${blockStyle}">`;
			}
			const styled = attributes.replace(
				/\bstyle\s*=\s*(["'])(.*?)\1/i,
				(_styleMatch, quote: string, value: string) => {
					const existing = value.trim().replace(/;?\s*$/, '');
					return `style=${quote}${existing ? `${existing};` : ''}${blockStyle}${quote}`;
				},
			);
			return `<${tag}${styled}>`;
		},
	);
}
