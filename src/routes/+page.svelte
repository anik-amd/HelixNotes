<script lang="ts">
	import { onDestroy, onMount, tick } from 'svelte';
	import { appConfig, vaultReady, theme } from '$lib/stores/app';
	import { getAppConfig, openVault, restoreExternalVault, setFontSize } from '$lib/api';
	import { darkThemes, isIOS, isMobile } from '$lib/platform';
	import { getWheelFontSizeAction } from '$lib/utils/editor-zoom';
	import { applyUiScale, defaultUiScale } from '$lib/ui-scale';
	import { listen } from '@tauri-apps/api/event';
	import { getCurrentWindow } from '@tauri-apps/api/window';
	import VaultPicker from '$lib/components/VaultPicker.svelte';
	import AppLayout from '$lib/components/AppLayout.svelte';
	import NoteWindow from '$lib/components/NoteWindow.svelte';

	let loading = $state(true);
	let noteWindowPath = $state<string | null>(null);
	let noteWindowVault = $state<string | null>(null);
	let startupVaultError = $state('');
	let fontSizeSaveTimer: ReturnType<typeof setTimeout> | null = null;
	let removeEditorZoomShortcuts: (() => void) | null = null;
	let removeFontSizeListener: (() => void) | null = null;
	let removeScrollFontSizeListener: (() => void) | null = null;
	let startupRevealTimer: ReturnType<typeof setTimeout> | null = null;
	let startupWindowRevealed = false;

	const STARTUP_REVEAL_FAILSAFE_MS = 4000;

	async function revealStartupWindow() {
		if (isMobile || startupWindowRevealed) return;
		await tick();
		// Force style resolution while the native window is still hidden so its first
		// compositor frame already uses the selected theme.
		getComputedStyle(document.body).backgroundColor;
		try {
			await getCurrentWindow().show();
			startupWindowRevealed = true;
			if (startupRevealTimer) {
				clearTimeout(startupRevealTimer);
				startupRevealTimer = null;
			}
		} catch {
			// Plain-browser development has no Tauri window to reveal.
		}
	}

	const defaultEditorFontSize = 14;
	const minEditorFontSize = 10;
	const maxEditorFontSize = 32;

	function getEditorScrollSurface(): HTMLElement | null {
		const sourceEditor = document.querySelector<HTMLElement>('.source-editor .cm-scroller');
		if (sourceEditor && sourceEditor.clientHeight > 0) return sourceEditor;
		return document.querySelector<HTMLElement>('.editor-body');
	}

	function preserveEditorViewportAfterLayout(scrollSurface: HTMLElement | null) {
		if (!scrollSurface) return () => {};

		const isSourceEditor = scrollSurface.classList.contains('cm-scroller');
		const maxScroll = scrollSurface.scrollHeight - scrollSurface.clientHeight;
		const scrollRatio = maxScroll > 0 ? scrollSurface.scrollTop / maxScroll : 0;
		const rect = scrollSurface.getBoundingClientRect();
		const anchorY = Math.min(rect.bottom - 1, rect.top + Math.max(16, rect.height * 0.25));
		let anchorElement: HTMLElement | null = null;
		let anchorOffset = 0;

		if (!isSourceEditor) {
			const candidate = document.elementFromPoint(rect.left + Math.min(80, rect.width / 2), anchorY);
			anchorElement = candidate?.closest('.tiptap *') as HTMLElement | null;
			if (anchorElement) anchorOffset = anchorElement.getBoundingClientRect().top - rect.top;
		}

		return () => {
			requestAnimationFrame(() => {
				requestAnimationFrame(() => {
					if (anchorElement && scrollSurface.contains(anchorElement)) {
						const nextRect = scrollSurface.getBoundingClientRect();
						scrollSurface.scrollTop += anchorElement.getBoundingClientRect().top - nextRect.top - anchorOffset;
					} else {
						const nextMaxScroll = scrollSurface.scrollHeight - scrollSurface.clientHeight;
						scrollSurface.scrollTop = Math.max(0, nextMaxScroll * scrollRatio);
					}
				});
			});
		};
	}

	function applyEditorFontSize(value: number) {
		const nextSize = Math.max(minEditorFontSize, Math.min(maxEditorFontSize, Math.round(value)));
		const restoreEditorViewport = preserveEditorViewportAfterLayout(getEditorScrollSurface());
		if ($appConfig) $appConfig.font_size = nextSize;
		document.documentElement.style.setProperty('--editor-font-size', `${nextSize}px`);
		restoreEditorViewport();
		return nextSize;
	}

	function setEditorFontSize(value: number) {
		const nextSize = applyEditorFontSize(value);
		if (fontSizeSaveTimer) clearTimeout(fontSizeSaveTimer);
		fontSizeSaveTimer = setTimeout(() => {
			setFontSize(nextSize).catch((e) => console.error('Failed to save font size:', e));
			fontSizeSaveTimer = null;
		}, 150);
	}

	function installEditorZoomShortcuts() {
		const zoomEditor = (delta: number) => setEditorFontSize(($appConfig?.font_size ?? defaultEditorFontSize) + delta);
		const handleKeydown = (event: KeyboardEvent) => {
			if (!event.ctrlKey && !event.metaKey) return;
			if (event.altKey || event.shiftKey) return;

			if (event.key === '+' || event.key === '=') {
				event.preventDefault();
				event.stopPropagation();
				zoomEditor(1);
			} else if (event.key === '-' || event.key === '_') {
				event.preventDefault();
				event.stopPropagation();
				zoomEditor(-1);
			} else if (event.key === '0') {
				event.preventDefault();
				event.stopPropagation();
				// Back to the size chosen in Appearance, not the built-in default.
				setEditorFontSize($appConfig?.preferred_font_size ?? defaultEditorFontSize);
			}
		};

		const handleWheel = (event: WheelEvent) => {
			const action = getWheelFontSizeAction(
				event,
				$appConfig?.scroll_to_change_font_size ?? true
			);
			if (action === 'ignore') return;
			event.preventDefault();
			event.stopPropagation();
			if (action === 'increase') zoomEditor(1);
			else if (action === 'decrease') zoomEditor(-1);
		};

		window.addEventListener('keydown', handleKeydown);
		window.addEventListener('wheel', handleWheel, { passive: false, capture: true });

		return () => {
			window.removeEventListener('keydown', handleKeydown);
			window.removeEventListener('wheel', handleWheel, { capture: true });
		};
	}

	onMount(async () => {
		if (!isMobile) {
			startupRevealTimer = setTimeout(() => void revealStartupWindow(), STARTUP_REVEAL_FAILSAFE_MS);
		}
		// Check for note window mode
		const params = new URLSearchParams(window.location.search);
		const notePath = params.get('note');
		if (notePath) {
			noteWindowPath = notePath;
			noteWindowVault = params.get('vault');
		}

		try {
			const config = await getAppConfig();
			if (noteWindowPath) {
				const origin = config.vaults.find(vault => vault.path === noteWindowVault);
				if (noteWindowVault && (origin || config.active_vault === noteWindowVault)) {
					// This is window-local state, not a native vault switch.
					config.active_vault = noteWindowVault;
					config.active_bookmark_id = origin?.bookmark_id ?? null;
				} else {
					noteWindowPath = null;
					noteWindowVault = null;
					startupVaultError = 'This note window’s vault is no longer available. Choose a vault to continue.';
				}
			}
			$appConfig = config;
			removeFontSizeListener = await listen<number>('editor-font-size-changed', (event) => {
				if ($appConfig?.font_size !== event.payload) applyEditorFontSize(event.payload);
			});
			removeScrollFontSizeListener = await listen<boolean>('scroll-to-change-font-size-changed', (event) => {
				if ($appConfig) $appConfig.scroll_to_change_font_size = event.payload;
			});
			$theme = config.theme || 'system';

			// Apply theme immediately to prevent flash. Runs before the stores settle, so resolve
			// "system" against the configured pair here the same way resolvedTheme does.
			const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
			const rawTheme = config.theme || 'system';
			const themeValue = rawTheme === 'system'
				? (prefersDark ? config.system_dark_theme || 'dark' : config.system_light_theme || 'light')
				: rawTheme;
			const root = document.documentElement;
			const namedThemes = ['solarized-light', 'solarized-dark', 'catppuccin', 'nord', 'tokyo-night', 'github-light', 'github-dark', 'dracula', 'blueberry', 'forest-green', 'gruvbox', 'midnight-tide', 'cherry-blossom', 'synthwave', 'ember', 'moonlit', 'light-coffee', 'dark-coffee', 'cotton-candy', 'crimson', 'cloud', 'peach', 'material-dark', 'material-light', 'monokai', 'rose-pine', 'everforest', 'horizon', 'cyberpunk', 'black', 'one-dark'];
			root.classList.remove('dark');
			root.removeAttribute('data-theme');
			if (namedThemes.includes(themeValue)) {
				root.setAttribute('data-theme', themeValue);
				if (darkThemes.includes(themeValue)) root.classList.add('dark');
			} else if (themeValue === 'dark') {
				root.classList.add('dark');
			}

			// Apply saved font settings
			if (config.font_size) applyEditorFontSize(config.font_size);
			if (config.line_height) {
				document.documentElement.style.setProperty('--editor-line-height', String(config.line_height));
			}
			if (config.content_width) {
				document.documentElement.style.setProperty('--editor-content-width', `${config.content_width}px`);
			}
			if (config.font_family) {
				const fontStacks: Record<string, string> = {
					system: '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif',
					inter: '"Inter", -apple-system, BlinkMacSystemFont, sans-serif',
					georgia: 'Georgia, "Times New Roman", serif',
					merriweather: '"Merriweather", Georgia, serif',
					lora: '"Lora", Georgia, serif',
					opensans: '"Open Sans", -apple-system, sans-serif',
					literata: '"Literata", Georgia, serif',
					mono: '"JetBrains Mono", "Fira Code", "Cascadia Code", monospace',
					ubuntu: '"Ubuntu", -apple-system, sans-serif',
					'ubuntu-mono': '"Ubuntu Mono", monospace',
				};
				const stack = fontStacks[config.font_family];
				if (stack) document.documentElement.style.setProperty('--editor-font-family', stack);
			}
			// Apply saved accent
			if (config.accent_color) {
				const customTheme = config.custom_themes.find(theme => theme.id === themeValue);
				const isDark = darkThemes.includes(themeValue) || (customTheme?.is_dark ?? false);
				let color: string | null = null;
				if (config.accent_color.startsWith('#')) {
					color = config.accent_color;
				} else {
					const accentPresets: Record<string, { light: string; dark: string }> = {
						Indigo: { light: '#5b6abf', dark: '#7b9bd4' },
						Rose: { light: '#e11d48', dark: '#d4768a' },
						Emerald: { light: '#059669', dark: '#5bab8a' },
						Amber: { light: '#d97706', dark: '#c9a04e' },
						Purple: { light: '#7c3aed', dark: '#9a82c4' },
						Cyan: { light: '#0891b2', dark: '#5ba8b5' },
						Orange: { light: '#ea580c', dark: '#c98560' },
						Teal: { light: '#0d9488', dark: '#5fada5' },
						Sakura: { light: '#d4547a', dark: '#f4a0b5' },
						Gold: { light: '#b8860b', dark: '#fabd2f' },
						Lavender: { light: '#9d5cdb', dark: '#c084fc' },
						Sky: { light: '#0284c7', dark: '#38bdf8' },
						Coral: { light: '#e85d4a', dark: '#ff8a7a' },
					};
					const preset = accentPresets[config.accent_color];
					if (preset) color = isDark ? preset.dark : preset.light;
				}
				if (color) {
					const root = document.documentElement;
					root.style.setProperty('--accent', color);
					root.style.setProperty('--text-accent', color);
					root.style.setProperty('--accent-hover', color);
					root.style.setProperty('--accent-light', isDark
						? `color-mix(in srgb, ${color} 10%, transparent)`
						: `color-mix(in srgb, ${color} 8%, transparent)`);
				}
			}

			// Apply saved interface scale (or the platform default) to desktop windows and Android.
			const uiScale = config.ui_scale ?? defaultUiScale;
			if (uiScale !== 1) await applyUiScale(uiScale);
			await revealStartupWindow();
			removeEditorZoomShortcuts = installEditorZoomShortcuts();

			// Auto-open last vault if available
			if (config.active_vault) {
				const activeVault = config.active_bookmark_id
					? config.vaults.find((vault) => vault.bookmark_id === config.active_bookmark_id)
					: config.vaults.find(
							(vault) => !vault.bookmark_id && vault.path === config.active_vault
						);
				try {
					// Note windows don't re-open the vault (already open in main process)
					if (!noteWindowPath) {
						if (isIOS && activeVault?.bookmark_id) {
							await restoreExternalVault(activeVault.bookmark_id);
							$appConfig = await getAppConfig();
						} else {
							await openVault(config.active_vault);
						}
					}
					$vaultReady = true;
				} catch (e) {
					$vaultReady = false;
					startupVaultError = `Could not open the last vault: ${String(e)}`;
					// A saved path is not proof that the vault was initialized.
					// Do not let the picker navigate back into a failed startup.
					$appConfig = {
						...config,
						active_vault: null,
						active_bookmark_id: null
					};
				}
			}
		} catch {
			// First launch or no config
		} finally {
			loading = false;
			await revealStartupWindow();
			if (startupRevealTimer) {
				clearTimeout(startupRevealTimer);
				startupRevealTimer = null;
			}
		}
	});

	onDestroy(() => {
		removeEditorZoomShortcuts?.();
		removeFontSizeListener?.();
		removeScrollFontSizeListener?.();
		if (fontSizeSaveTimer) clearTimeout(fontSizeSaveTimer);
		if (startupRevealTimer) clearTimeout(startupRevealTimer);
	});
</script>

{#if loading}
	<div class="loading">
		<div class="spinner"></div>
	</div>
{:else if noteWindowPath}
	<NoteWindow notePath={noteWindowPath} vaultPath={noteWindowVault} />
{:else if $vaultReady}
	<AppLayout />
{:else}
	<VaultPicker initialError={startupVaultError} />
{/if}

<style>
	.loading {
		display: flex;
		align-items: center;
		justify-content: center;
		height: 100vh;
		background: var(--bg-primary);
	}

	.spinner {
		width: 24px;
		height: 24px;
		border: 2px solid var(--border-color);
		border-top-color: var(--accent);
		border-radius: 50%;
		animation: spin 0.6s linear infinite;
	}

	@keyframes spin {
		to {
			transform: rotate(360deg);
		}
	}
</style>
