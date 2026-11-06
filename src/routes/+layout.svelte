<script lang="ts">
	import { onMount } from 'svelte';
	import '../app.css';
	import { resolvedTheme, appConfig, activeNotePath, installType, platformIsMobile, checkForUpdate, checkForUpdateMobile, isManagedInstall, customThemes } from '$lib/stores/app';
	import { openFile, openUrl, getInstallType, isMobilePlatform } from '$lib/api';
	import { get } from 'svelte/store';
	import { darkThemes, isMobile, isAndroid } from '$lib/platform';
	import type { CustomTheme } from '$lib/types';
	import ResizeHandles from '$lib/components/ResizeHandles.svelte';
	import { noteAssetRoot, resolveFileReference } from '$lib/utils/paths';

	let { children } = $props();

	// Reactively apply theme class to <html> whenever the resolved theme or custom themes change.
	// $resolvedTheme already maps "system" onto the configured light/dark pair, so flipping the OS
	// appearance re-runs this effect.
	$effect(() => {
		applyTheme($resolvedTheme, $customThemes);
	});

	// Apply link arrow visibility from config
	$effect(() => {
		if ($appConfig) {
			document.documentElement.classList.toggle('no-link-arrows', !$appConfig.show_link_arrows);
		}
	});

	const CUSTOM_THEME_VARS = [
		'--bg-primary', '--bg-secondary', '--bg-tertiary',
		'--bg-hover', '--bg-active', '--bg-editor',
		'--text-primary', '--text-secondary', '--border-color',
		'--border-light', '--text-tertiary',
	];

	// A custom theme carries nine colors, but the stylesheet also reads --border-light and
	// --text-tertiary. Named themes set those in their own :root[data-theme] block; a custom theme
	// has no such block, so without these they fall back to the light defaults in app.css and a dark
	// custom theme draws near-white dividers. Derive them from the closest color the theme does have.
	function applyCustomThemeVars(root: HTMLElement, ct: CustomTheme) {
		root.style.setProperty('--bg-primary', ct.colors.bg_primary);
		root.style.setProperty('--bg-secondary', ct.colors.bg_secondary);
		root.style.setProperty('--bg-tertiary', ct.colors.bg_tertiary);
		root.style.setProperty('--bg-hover', ct.colors.bg_hover);
		root.style.setProperty('--bg-active', ct.colors.bg_active);
		root.style.setProperty('--bg-editor', ct.colors.bg_editor);
		root.style.setProperty('--text-primary', ct.colors.text_primary);
		root.style.setProperty('--text-secondary', ct.colors.text_secondary);
		root.style.setProperty('--border-color', ct.colors.border_color);
		root.style.setProperty('--border-light', ct.colors.border_color);
		root.style.setProperty('--text-tertiary', ct.colors.text_secondary);
	}

	function clearCustomThemeVars(root: HTMLElement) {
		for (const v of CUSTOM_THEME_VARS) root.style.removeProperty(v);
	}

	function applyTheme(t: string, themes: CustomTheme[] = []) {
		const namedThemes = ['solarized-light', 'solarized-dark', 'catppuccin', 'nord', 'tokyo-night', 'github-light', 'github-dark', 'dracula', 'blueberry', 'forest-green', 'gruvbox', 'midnight-tide', 'cherry-blossom', 'synthwave', 'ember', 'moonlit', 'light-coffee', 'dark-coffee', 'cotton-candy', 'crimson', 'cloud', 'peach', 'material-dark', 'material-light', 'monokai', 'rose-pine', 'everforest', 'horizon', 'cyberpunk', 'black', 'one-dark', 'matrix', 'amber-crt', 'sepia', 'deep-ocean', 'sandstone', 'lavender', 'graphite', 'nebula', 'wine', 'jade'];
		const root = document.documentElement;
		root.classList.remove('dark');
		root.removeAttribute('data-theme');
		clearCustomThemeVars(root);
		if (t.startsWith('custom-')) {
			const ct = themes.find(c => c.id === t);
			if (ct) {
				applyCustomThemeVars(root, ct);
				if (ct.is_dark) root.classList.add('dark');
			}
		} else if (namedThemes.includes(t)) {
			root.setAttribute('data-theme', t);
			if (darkThemes.includes(t)) root.classList.add('dark');
		} else if (t === 'dark' || (t === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches)) {
			root.classList.add('dark');
		}
	}

	function openLocalFile(path: string) {
		if (isAndroid) {
			const bridge = (window as any).Android;
			if (bridge && typeof bridge.openFile === 'function') {
				bridge.openFile(path);
			}
		} else {
			openFile(path).catch((err) => console.error('Failed to open file:', err));
		}
	}

	function resolveAndHandleLink(href: string) {
		if (href.startsWith('http://') || href.startsWith('https://') || href.startsWith('mailto:') || href.startsWith('tel:') || href.startsWith('sms:')) {
			openUrl(href).catch((err) => console.error('Failed to open URL:', err));
		} else if (!href.startsWith('#')) {
			const config = get(appConfig);
			const note = get(activeNotePath);
			const root = noteAssetRoot(note, config?.active_vault ?? null, config?.vaults.map(vault => vault.path) ?? []);
			const absPath = resolveFileReference(href, note, root).path;
			// Internal .md note link - navigate within the app
			if (absPath.endsWith('.md')) {
				window.dispatchEvent(new CustomEvent('helixnotes-open-note', { detail: absPath }));
			} else {
				openLocalFile(absPath);
			}
		}
	}

	// Detect install type and check for updates on startup
	onMount(() => {
		// Authoritative platform from the backend (compile-time), overriding the UA guess. (#63)
		isMobilePlatform().then((m) => platformIsMobile.set(m)).catch(() => {});
		if (isMobile) {
			installType.set('android');
			// Android updates come from the F-droid repo / Obtainium, so no in-app update check.
		} else {
			getInstallType().then(t => {
				installType.set(t);
				if (!isManagedInstall(t)) checkForUpdate();
			}).catch(() => {});
		}
	});

	// Intercept all link clicks in capture phase to prevent webview navigation
	onMount(() => {
		function handleLinkClick(e: MouseEvent) {
			const el = e.target as HTMLElement;
			// PDF/attachment embeds: check for data-open-file on target or ancestors
			const fileTarget = el?.closest('[data-open-file]') as HTMLElement | null;
			if (fileTarget) {
				e.preventDefault();
				e.stopPropagation();
				e.stopImmediatePropagation();
				openLocalFile(fileTarget.getAttribute('data-open-file')!);
				return;
			}
			const target = el?.closest('a');
			if (!target) return;
			const href = target.getAttribute('href');
			if (!href) return;
			e.preventDefault();
			e.stopPropagation();
			e.stopImmediatePropagation();
			resolveAndHandleLink(href);
		}

		document.addEventListener('click', handleLinkClick, true);

		// On Android WebView, tapping a link can trigger native navigation before
		// the JS click event fires (causing 404s for relative .md links).
		// We intercept touchend: if it's a tap (not a scroll) on an <a>, we
		// preventDefault to block the native navigation and handle it ourselves.
		let touchStartY = 0;
		function handleTouchStart(e: TouchEvent) {
			touchStartY = e.touches[0]?.clientY ?? 0;
		}
		function handleTouchEnd(e: TouchEvent) {
			const touch = e.changedTouches[0];
			if (!touch) return;
			const endY = touch.clientY;
			// If the finger moved > 10px, it's a scroll, not a tap
			if (Math.abs(endY - touchStartY) > 10) return;
			// Android WebView may report e.target as the editor root for elements
			// inside contenteditable="false" blocks; use elementFromPoint instead
			const el = (document.elementFromPoint(touch.clientX, touch.clientY) ?? e.target) as HTMLElement;
			// PDF/attachment embeds: check for data-open-file on target or ancestors
			const fileTarget = el?.closest('[data-open-file]') as HTMLElement | null;
			if (fileTarget) {
				e.preventDefault();
				openLocalFile(fileTarget.getAttribute('data-open-file')!);
				return;
			}
			const target = el?.closest('a');
			if (!target) return;
			const href = target.getAttribute('href');
			if (!href) return;
			e.preventDefault();
			resolveAndHandleLink(href);
		}

		if (isMobile) {
			document.addEventListener('touchstart', handleTouchStart, { capture: true, passive: true });
			document.addEventListener('touchend', handleTouchEnd, true);
		}

		return () => {
			document.removeEventListener('click', handleLinkClick, true);
			if (isMobile) {
				document.removeEventListener('touchstart', handleTouchStart, true);
				document.removeEventListener('touchend', handleTouchEnd, true);
			}
		};
	});

	// On Windows the native OS drag-drop handler is disabled (dragDropEnabled:false
	// in tauri.windows.conf.json) so HTML5 drag-and-drop works for reordering. With it
	// off, a file dropped outside a drop zone would make the webview navigate to / open
	// that file, replacing the app. Swallow any drag that bubbles up unhandled. Real drop
	// zones (editor, sidebar reordering) call preventDefault in their own handlers first;
	// these bubble-phase listeners only act as a fallback. On macOS/Linux OS file drops are
	// intercepted natively and never surface as HTML5 events, so this is a harmless no-op there.
	onMount(() => {
		function preventNavigate(e: DragEvent) {
			e.preventDefault();
		}
		window.addEventListener('dragover', preventNavigate);
		window.addEventListener('drop', preventNavigate);
		return () => {
			window.removeEventListener('dragover', preventNavigate);
			window.removeEventListener('drop', preventNavigate);
		};
	});
</script>

<svelte:document oncontextmenu={(e) => { if (!isMobile) e.preventDefault(); }} />

<svelte:head>
	<title>HelixNotes</title>
</svelte:head>

{@render children()}

<ResizeHandles />
