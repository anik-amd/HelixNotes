// Platform is the compile-time build target, injected by the backend into
// window.__HELIX_PLATFORM__ before app scripts run (see src-tauri/src/lib.rs). The UA is only a
// fallback when that global is absent (SSR/prerender, plain-browser dev): some desktop WebKitGTK
// builds report a mobile-looking user-agent. (#63)
type HelixPlatform = { mobile: boolean; android: boolean; ios: boolean };

const injected: HelixPlatform | undefined =
	typeof window !== 'undefined'
		? (window as unknown as { __HELIX_PLATFORM__?: HelixPlatform }).__HELIX_PLATFORM__
		: undefined;

const ua = typeof navigator !== 'undefined' ? navigator.userAgent : '';

export const isAndroid = injected ? injected.android : /android/i.test(ua);
export const isIOS = injected ? injected.ios : /iphone|ipad|ipod/i.test(ua);
export const isMobile = injected ? injected.mobile : isAndroid || isIOS;

// Themes that use the dark color scheme. Used by applyTheme() to toggle the
// `dark` class on the root element. Keep this in sync when adding new themes.
export const darkThemes = [
	'dark',
	'solarized-dark',
	'catppuccin',
	'nord',
	'tokyo-night',
	'github-dark',
	'dracula',
	'blueberry',
	'forest-green',
	'gruvbox',
	'midnight-tide',
	'cherry-blossom',
	'synthwave',
	'ember',
	'moonlit',
	'dark-coffee',
	'crimson',
	'material-dark',
	'monokai',
	'rose-pine',
	'everforest',
	'horizon',
	'cyberpunk',
	'black',
	'one-dark',
	'matrix',
	'amber-crt',
	'deep-ocean',
	'graphite',
	'nebula',
	'wine',
	'jade',
];

export function isDarkTheme(theme: string): boolean {
	return darkThemes.includes(theme) || (theme === 'system' && typeof window !== 'undefined' && window.matchMedia('(prefers-color-scheme: dark)').matches);
}
