import { isValidTagColor } from './tag-styles.ts';

export const NOTEBOOK_ICON_OPTIONS = [
  { id: 'book', label: 'Book' },
  { id: 'folder', label: 'Folder' },
  { id: 'briefcase', label: 'Work' },
  { id: 'home', label: 'Home' },
  { id: 'star', label: 'Star' },
  { id: 'heart', label: 'Heart' },
  { id: 'lightbulb', label: 'Ideas' },
  { id: 'code', label: 'Code' },
  { id: 'flask', label: 'Research' },
  { id: 'graduation', label: 'Study' },
  { id: 'calendar', label: 'Calendar' },
  { id: 'tasks', label: 'Tasks' },
  { id: 'archive', label: 'Archive' },
  { id: 'globe', label: 'Globe' },
  { id: 'plane', label: 'Travel' },
  { id: 'palette', label: 'Creative' },
  { id: 'music', label: 'Music' },
  { id: 'camera', label: 'Photos' },
  { id: 'film', label: 'Film' },
  { id: 'gamepad', label: 'Games' },
  { id: 'cart', label: 'Shopping' },
  { id: 'money', label: 'Finance' },
  { id: 'health', label: 'Health' },
  { id: 'food', label: 'Food' },
  { id: 'coffee', label: 'Coffee' },
  { id: 'leaf', label: 'Nature' },
  { id: 'users', label: 'People' },
  { id: 'chat', label: 'Chat' },
  { id: 'mail', label: 'Mail' },
  { id: 'lock', label: 'Private' },
  { id: 'wrench', label: 'Tools' },
  { id: 'pen', label: 'Writing' }
] as const;

export type NotebookIconId = (typeof NOTEBOOK_ICON_OPTIONS)[number]['id'];

const BUILTIN_PREFIX = 'builtin:';
const BUILTIN_IDS: Record<NotebookIconId, true> = {
  book: true,
  folder: true,
  briefcase: true,
  home: true,
  star: true,
  heart: true,
  lightbulb: true,
  code: true,
  flask: true,
  graduation: true,
  calendar: true,
  tasks: true,
  archive: true,
  globe: true,
  plane: true,
  palette: true,
  music: true,
  camera: true,
  film: true,
  gamepad: true,
  cart: true,
  money: true,
  health: true,
  food: true,
  coffee: true,
  leaf: true,
  users: true,
  chat: true,
  mail: true,
  lock: true,
  wrench: true,
  pen: true
};

export function normalizeNotebookIconKey(path: string): string {
  return path.replace(/\\/g, '/');
}

export function encodeBuiltinNotebookIcon(icon: NotebookIconId, color?: string | null): string {
  return `${BUILTIN_PREFIX}${icon}${isValidTagColor(color) ? color.toLowerCase() : ''}`;
}

export function decodeBuiltinNotebookIcon(value: string | null | undefined): NotebookIconId | null {
  if (!value?.startsWith(BUILTIN_PREFIX)) return null;
  const icon = value.slice(BUILTIN_PREFIX.length);
  return Object.hasOwn(BUILTIN_IDS, icon) ? icon as NotebookIconId : null;
}

/** Notebook icons keep their color in the stored value: `builtin:star#e11d48`. */
export function decodeNotebookIcon(
  value: string | null | undefined
): { icon: NotebookIconId; color: string | null } | null {
  const hash = value?.indexOf('#') ?? -1;
  if (hash < 0) {
    const icon = decodeBuiltinNotebookIcon(value);
    return icon ? { icon, color: null } : null;
  }
  const icon = decodeBuiltinNotebookIcon(value!.slice(0, hash));
  const color = value!.slice(hash);
  return icon ? { icon, color: isValidTagColor(color) ? color : null } : null;
}
