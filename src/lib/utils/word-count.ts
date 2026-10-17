/** Words are runs of non-space text with at least one letter or digit, so `|`, `-` or `>` don't count. */
export function countWords(text: string): number {
	let count = 0;
	for (const token of text.split(/\s+/)) {
		if (/[\p{L}\p{N}]/u.test(token)) count++;
	}
	return count;
}

/** The readable text of a Markdown note, for counting words and characters in source mode. */
export function markdownCountText(markdown: string): string {
	return markdown
		.replace(/^---\n[\s\S]*?\n---\n?/, '')
		// Table delimiter rows (`| --- | :---: |`) are layout, not words.
		.replace(/^[ \t]*\|?[ \t]*:?-{3,}:?[ \t]*(?:\|[ \t]*:?-{3,}:?[ \t]*)*\|?[ \t]*$/gm, '')
		.replace(/!\[[^\]]*\]\([^)]*\)/g, '')
		.replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
		.replace(/\[\[([^\]|]*\|)?([^\]]*)\]\]/g, '$2')
		.replace(/^[ \t]*#{1,6}[ \t]+/gm, '')
		.replace(/^[ \t]*(?:>[ \t]*)+/gm, '')
		.replace(/^[ \t]*(?:[-*+]|\d+[.)])[ \t]+(?:\[[ xX]\][ \t]+)?/gm, '')
		.replace(/[*_~`|]+/g, ' ');
}
