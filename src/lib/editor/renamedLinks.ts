/*
 * Renaming a note rewrites the links to it in every note that links there (see
 * `update_wikilinks_after_rename` in the backend). A note that is open with unsaved edits has to
 * follow in the editor, or its next save would write the old links back. These rules mirror the
 * backend's; both are checked against `tests/fixtures/renamed-links.json`.
 */

export interface LinkRename {
	/** Vault-relative path of the note without `.md`, before and after. */
	old_ref: string;
	new_ref: string;
	old_title: string;
	new_title: string;
	/** Links written with the title follow (the title changed and no other note has it). */
	title_rules: boolean;
	/** No other note has this file name, so a bare file name or top-level path means this note. */
	file_rules?: boolean;
}

export interface RenamedLink {
	ref: string;
	display: string | null;
}

const lower = (value: string) => value.toLowerCase();

function splitRef(ref: string): [string, string] {
	const slash = ref.lastIndexOf('/');
	return slash < 0 ? ['', ref] : [ref.slice(0, slash), ref.slice(slash + 1)];
}

const joinRef = (dir: string, name: string) => (dir ? `${dir}/${name}` : name);

/** `name` without a trailing `.md` (any capitals), and that extension as written. */
function splitMdExt(name: string): [string, string] {
	return /\.md$/i.test(name) ? [name.slice(0, -3), name.slice(-3)] : [name, ''];
}

const byPath = (renames: readonly LinkRename[], ref: string) =>
	renames.find((rename) => lower(rename.old_ref) === lower(ref));

/**
 * The wiki link `[[ref]]` or `[[ref|display]]` after the renames, or null when it is not affected.
 * `ref` may carry a heading or block (`Note#Part`, `Note^id`) and a `.md` extension; both are kept.
 */
export function renamedLink(ref: string, display: string | null, renames: LinkRename | readonly LinkRename[]): RenamedLink | null {
	const list = Array.isArray(renames) ? renames : [renames as LinkRename];
	const anchorAt = ref.search(/[#^]/);
	const name = anchorAt < 0 ? ref : ref.slice(0, anchorAt);
	const anchor = anchorAt < 0 ? '' : ref.slice(anchorAt);
	const [base, ext] = splitMdExt(name);
	let rename = list.find((candidate) => candidate.title_rules && lower(candidate.old_title) === lower(base));
	let next = rename?.new_title;
	if (!rename) {
		rename = byPath(list, base.replace(/\\/g, '/'));
		if (!rename || rename.old_ref === rename.new_ref) return null;
		const stemChanges = splitRef(rename.old_ref)[1] !== splitRef(rename.new_ref)[1];
		// At the top of the vault the path is the bare name, as unsure as a title link.
		if (!rename.old_ref.includes('/') && !(rename.file_rules && stemChanges)) return null;
		next = rename.new_ref;
	}
	const followsTitle = display === rename.old_title && rename.old_title !== rename.new_title;
	return { ref: next + ext + anchor, display: followsTitle ? rename.new_title : display };
}

/** Resolve `.` and `..` in a slash path; null when it climbs out of the vault. */
function normalizeDir(path: string): string | null {
	const parts: string[] = [];
	for (const part of path.split('/')) {
		if (!part || part === '.') continue;
		if (part === '..') {
			if (!parts.length) return null;
			parts.pop();
		} else parts.push(part);
	}
	return parts.join('/');
}

/** The `../`-style prefix that leads from folder `from` to folder `to`, ending in `/` unless empty. */
function relativeDir(from: string, to: string): string {
	const a = from.split('/').filter(Boolean);
	const b = to.split('/').filter(Boolean);
	let common = 0;
	while (common < a.length && common < b.length && a[common] === b[common]) common++;
	return '../'.repeat(a.length - common) + b.slice(common).map((part) => `${part}/`).join('');
}

function decode(value: string): string {
	try {
		return decodeURIComponent(value);
	} catch {
		return value;
	}
}

/** Characters a link destination without `<>` cannot hold as they are. */
function encodeFileName(name: string): string {
	return name.replace(/[% \t()<>#]/g, (char) => `%${char.charCodeAt(0).toString(16).toUpperCase().padStart(2, '0')}`);
}

const encodePath = (path: string, angled: boolean) => (angled ? path : path.split('/').map(encodeFileName).join('/'));

/**
 * The destination of a Markdown link (`Note.md`, `../Folder/Note%20name.md#Part`, `<Note name.md>`)
 * after the renames, or null when it does not point at a renamed note. `noteDir` is the
 * vault-relative folder of the note holding the link. The link keeps its form: relative stays
 * relative, from the vault root stays so, and a bare name stays bare when it still finds the note.
 */
export function renamedHref(href: string, noteDir: string, renames: LinkRename | readonly LinkRename[]): string | null {
	const list = Array.isArray(renames) ? renames : [renames as LinkRename];
	const angled = href.length >= 2 && href.startsWith('<') && href.endsWith('>');
	const dest = angled ? href.slice(1, -1) : href;
	if (/^[a-z][a-z0-9+.-]*:/i.test(dest) || !dest || dest.startsWith('#')) return null;
	const hashAt = dest.indexOf('#');
	const path = hashAt < 0 ? dest : dest.slice(0, hashAt);
	const anchor = hashAt < 0 ? '' : dest.slice(hashAt);
	const slash = path.lastIndexOf('/');
	const rawDir = slash < 0 ? '' : path.slice(0, slash + 1);
	const rawFile = path.slice(slash + 1);
	const [, ext] = splitMdExt(rawFile);
	if (!ext) return null;
	const [stem] = splitMdExt(decode(rawFile));
	const dir = decode(rawDir).replace(/\\/g, '/');
	let rename: LinkRename | undefined;
	let form: 'bare' | 'relative' | 'root' = 'relative';
	if (!dir) {
		rename = byPath(list, joinRef(noteDir, stem));
		if (!rename) {
			// A bare name from another folder finds the note by its unique file name.
			rename = list.find((candidate) => {
				const [, oldStem] = splitRef(candidate.old_ref);
				return candidate.file_rules && lower(oldStem) === lower(stem) && oldStem !== splitRef(candidate.new_ref)[1];
			});
			form = 'bare';
		}
	} else {
		const relative = dir.startsWith('/') ? null : normalizeDir(`${noteDir}/${dir}`);
		rename = relative === null ? undefined : byPath(list, joinRef(relative, stem));
		if (!rename) {
			const root = normalizeDir(dir);
			rename = root === null ? undefined : byPath(list, joinRef(root, stem));
			form = 'root';
		}
	}
	if (!rename || rename.old_ref === rename.new_ref) return null;
	const [oldDir] = splitRef(rename.old_ref);
	const [newDir, newStem] = splitRef(rename.new_ref);
	const file = encodePath(newStem + ext, angled);
	const prefix = lower(oldDir) === lower(newDir)
		? rawDir
		: form === 'bare'
			? ''
			: form === 'relative'
				? encodePath(relativeDir(noteDir, newDir), angled)
				: (dir.startsWith('/') ? '/' : '') + (newDir ? `${encodePath(newDir, angled)}/` : '');
	const next = prefix + file + anchor;
	return angled ? `<${next}>` : next;
}

export interface TextChange {
	from: number;
	to: number;
	insert: string;
}

/** Ranges of `text` outside fenced code blocks and inline code, where links are links. */
function proseRanges(text: string): [number, number][] {
	const ranges: [number, number][] = [];
	let fence: { char: string; length: number } | null = null;
	let offset = 0;
	for (const line of text.split(/(?<=\n)/)) {
		const start = offset;
		offset += line.length;
		const opening = /^ {0,3}(`{3,}|~{3,})(.*)$/.exec(line.replace(/\r?\n$/, ''));
		if (fence) {
			if (opening && opening[1][0] === fence.char && opening[1].length >= fence.length && !opening[2].trim()) fence = null;
			continue;
		}
		if (opening && !(opening[1][0] === '`' && opening[2].includes('`'))) {
			fence = { char: opening[1][0], length: opening[1].length };
			continue;
		}
		let from = start;
		const ticks = /`+/g;
		let match: RegExpExecArray | null;
		while ((match = ticks.exec(line))) {
			const closing = new RegExp(`(?<!\`)${match[0]}(?!\`)`, 'g');
			closing.lastIndex = match.index + match[0].length;
			const end = closing.exec(line);
			if (!end) continue;
			ranges.push([from, start + match.index]);
			from = start + end.index + end[0].length;
			ticks.lastIndex = end.index + end[0].length;
		}
		ranges.push([from, offset]);
	}
	return ranges;
}

const WIKI_LINK = /\[\[([^[\]\n]+?)\]\]/g;
const MARKDOWN_LINK = /\[([^[\]\n]*)\]\((<[^<>\n]*>|(?:[^\s()<>]|\([^\s()<>]*\))+)([^)\n]*)\)/g;

/** The edits that bring the wiki and Markdown links in Markdown text in line with the rename. */
export function renamedLinkChanges(text: string, renames: LinkRename | readonly LinkRename[], noteDir = ''): TextChange[] {
	const list = Array.isArray(renames) ? renames : [renames as LinkRename];
	const changes: TextChange[] = [];
	for (const [from, to] of proseRanges(text)) {
		const prose = text.slice(from, to);
		for (const match of prose.matchAll(WIKI_LINK)) {
			const inner = match[1];
			// In a table cell the alias separator is written `\|`, since a bare `|` ends the cell.
			const bar = inner.indexOf('|');
			const escaped = bar > 0 && inner[bar - 1] === '\\';
			const ref = bar < 0 ? inner : inner.slice(0, escaped ? bar - 1 : bar);
			const display = bar < 0 ? null : inner.slice(bar + 1);
			const next = renamedLink(ref, display, list);
			if (!next) continue;
			const separator = escaped ? '\\|' : '|';
			const insert = next.display === null ? `[[${next.ref}]]` : `[[${next.ref}${separator}${next.display}]]`;
			if (insert !== match[0]) changes.push({ from: from + match.index!, to: from + match.index! + match[0].length, insert });
		}
		for (const match of prose.matchAll(MARKDOWN_LINK)) {
			const href = renamedHref(match[2], noteDir, list);
			if (href === null) continue;
			// Link text that was the old title follows the new one.
			const titled = list.find((rename) => rename.old_title === match[1] && rename.old_title !== rename.new_title);
			const label = titled ? titled.new_title : match[1];
			const insert = `[${label}](${href}${match[3]})`;
			if (insert !== match[0]) changes.push({ from: from + match.index!, to: from + match.index! + match[0].length, insert });
		}
	}
	return changes.sort((a, b) => a.from - b.from);
}
