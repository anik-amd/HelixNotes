import type { FileEvent, NoteEntry } from '../types';

const normalize = (path: string) => path.includes('\\') ? path.replace(/\\/g, '/') : path;

/** A delayed watcher/sync batch must not update a different (even nested) vault. */
export function isCurrentVaultEvent(event: FileEvent, vaultPath: string | null): boolean {
  if (!vaultPath || typeof event.vault_path !== 'string') return false;
  return normalize(event.vault_path).replace(/\/+$/, '') === normalize(vaultPath).replace(/\/+$/, '');
}

export function sameNoteTags(left: NoteEntry, right: NoteEntry): boolean {
  return left.meta.tags.length === right.meta.tags.length &&
    left.meta.tags.every((tag) => right.meta.tags.includes(tag));
}

function belongsToList(note: NoteEntry, key: string, quickAccess: string[]): boolean {
  if (key.startsWith('tag:')) return note.meta.tags.includes(key.slice(4));
  if (key.startsWith('nb:')) {
    const path = normalize(note.path);
    return path.slice(0, path.lastIndexOf('/')) === normalize(key.slice(3));
  }
  if (key === 'daily') return /^Daily\/[^/]+$/.test(normalize(note.relative_path));
  if (key === 'quickaccess') return quickAccess.some((path) => normalize(path) === normalize(note.relative_path));
  return key === 'all';
}

/** Patch one note without rescanning the vault, including changes to tag membership. */
export function updateNoteList(list: NoteEntry[], note: NoteEntry, key: string, quickAccess: string[]): NoteEntry[] {
  if (key === 'trash') return list;
  const notePath = normalize(note.path);
  const index = list.findIndex((entry) => normalize(entry.path) === notePath);
  if (!belongsToList(note, key, quickAccess)) {
    return index < 0 ? list : list.filter((_, i) => i !== index);
  }
  if (index < 0) return [...list, note];
  const previous = list[index];
  if (previous.preview === note.preview && previous.relative_path === note.relative_path &&
      previous.meta.id === note.meta.id && previous.meta.title === note.meta.title &&
      previous.meta.pinned === note.meta.pinned && previous.meta.created === note.meta.created &&
      previous.meta.modified === note.meta.modified && sameNoteTags(previous, note)) return list;
  const updated = list.slice();
  updated[index] = note;
  return updated;
}
