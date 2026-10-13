<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { open as openDialog } from '@tauri-apps/plugin-dialog';
	import { readFile } from '@tauri-apps/plugin-fs';
	import { get } from 'svelte/store';
	import { appConfig, tagStyles } from '$lib/stores/app';
	import { importTagIcon, setTagStyle } from '$lib/api';
	import type { TagStyle, TagStyleChange } from '$lib/types';
	import { tagAppearance } from '$lib/utils/appearance-mutations';
	import { isMobile } from '$lib/platform';
	import NotebookGlyph from './NotebookGlyph.svelte';
	import TagLabel from './TagLabel.svelte';
	import {
		NOTEBOOK_ICON_OPTIONS,
		COMMON_NOTEBOOK_ICON_COUNT,
		decodeBuiltinNotebookIcon,
		isCommonNotebookIcon,
		encodeBuiltinNotebookIcon,
		type NotebookIconId
	} from '$lib/utils/notebook-icons';
	import {
		TAG_COLOR_PRESETS,
		applyCommittedTagStyle,
		lookupTagStyle,
		tagColor
	} from '$lib/utils/tag-styles';

	let {
		tag,
		onclose
	}: {
		tag: string;
		onclose: () => void;
	} = $props();

	let dialogEl = $state<HTMLDivElement | null>(null);
	const ownerVault = get(appConfig)?.active_vault ?? null;
	let disposed = false;
	let error = $state('');
	const ownerIsCurrent = () => !disposed && get(appConfig)?.active_vault === ownerVault;
	onDestroy(() => { disposed = true; });
	const current = $derived(lookupTagStyle(tag, $tagStyles));
	const activeIcon = $derived(current?.icon ?? null);
	const activeColor = $derived(tagColor(tag, $tagStyles));
	const colorInputValue = $derived(expandHex(activeColor) ?? '#5b6abf');
	const hasStyle = $derived(!!(activeIcon || activeColor));
	let showAll = $state<boolean | null>(null);
	// Starts open when the current icon is one of the extra icons.
	const expanded = $derived(showAll ?? !isCommonNotebookIcon(decodeBuiltinNotebookIcon(activeIcon)));
	function publishStyle(target: string, next: TagStyle | null) {
		if (get(appConfig)?.active_vault !== ownerVault) return;
		tagStyles.update(styles => applyCommittedTagStyle(styles, target, next));
	}

	onMount(() => dialogEl?.focus());

	function expandHex(color: string | null): string | null {
		if (!color) return null;
		if (color.length === 4) {
			return `#${color[1]}${color[1]}${color[2]}${color[2]}${color[3]}${color[3]}`;
		}
		return color;
	}

	function baseOf(path: string): string {
		return path.slice(Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\')) + 1);
	}

	async function persist(change: TagStyleChange): Promise<boolean> {
		if (!ownerVault || !ownerIsCurrent()) return false;
		const target = tag;
		const key = target.trim().toLowerCase();
		let latest: () => boolean;
		if (change.kind === 'reset') {
			tagAppearance.choose(ownerVault, `${key}:icon`);
			tagAppearance.choose(ownerVault, `${key}:color`);
			latest = () => true;
		} else {
			latest = tagAppearance.choose(ownerVault, `${key}:${change.kind}`);
		}
		error = '';
		try {
			await tagAppearance.run(ownerVault, key, latest,
				() => setTagStyle(ownerVault, target, change),
				(next) => publishStyle(target, next));
			return true;
		} catch (cause) {
			if (ownerIsCurrent() && tag === target && latest()) error = String(cause);
			return false;
		}
	}

	function handleBuiltinIcon(icon: NotebookIconId) {
		void persist({ kind: 'icon', value: encodeBuiltinNotebookIcon(icon) });
	}

	async function handleCustomIcon() {
		if (!ownerVault || !ownerIsCurrent()) return;
		const target = tag;
		const key = target.trim().toLowerCase();
		const latest = tagAppearance.choose(ownerVault, `${key}:icon`);
		error = '';
		try {
			const selected = await openDialog({
				multiple: false,
				filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'gif', 'svg', 'webp', 'ico'] }]
			});
			if (!selected || !ownerIsCurrent() || tag !== target || !latest()) return;
			const filePath = selected as string;
			const data = await readFile(filePath);
			if (!ownerIsCurrent() || tag !== target || !latest()) return;
			const fileName = baseOf(filePath) || 'icon.png';
			await tagAppearance.run(ownerVault, key, latest,
				() => importTagIcon(ownerVault, target, `tag-icon-${fileName}`, Array.from(data)),
				(next) => publishStyle(target, next));
		} catch (cause) {
			if (ownerIsCurrent() && tag === target && latest()) error = String(cause);
		}
	}

	async function handleReset() {
		const target = tag;
		if (await persist({ kind: 'reset' }) && ownerIsCurrent() && tag === target) onclose();
	}

	function onColorInput(e: Event) {
		void persist({ kind: 'color', value: (e.currentTarget as HTMLInputElement).value });
	}
</script>

<svelte:window
	onkeydown={(e) => {
		if (e.key === 'Escape') {
			e.preventDefault();
			onclose();
		}
	}}
/>

<div class="icon-picker-overlay" class:mobile={isMobile}>
	<button class="icon-picker-backdrop" aria-label="Close tag appearance picker" onclick={onclose}></button>
	<div
		bind:this={dialogEl}
		class="icon-picker"
		role="dialog"
		aria-modal="true"
		aria-label={`Choose an icon and color for ${tag}`}
		tabindex="-1"
	>
		<header class="icon-picker-header">
			<div>
				<h3>Tag appearance</h3>
				<p>
					<TagLabel name={tag} size={14} />
				</p>
			</div>
			<button type="button" class="icon-picker-close" aria-label="Close tag appearance picker" onclick={onclose}>
				<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M18 6 6 18M6 6l12 12" /></svg>
			</button>
		</header>
		{#if error}<div class="picker-error" role="alert">{error}</div>{/if}
		<div class="icon-picker-grid">
			{#each expanded ? NOTEBOOK_ICON_OPTIONS : NOTEBOOK_ICON_OPTIONS.slice(0, COMMON_NOTEBOOK_ICON_COUNT) as option (option.id)}
				<button
					type="button"
					class="icon-picker-option"
					class:active={activeIcon === encodeBuiltinNotebookIcon(option.id)}
					aria-label={option.label}
					title={option.label}
					onclick={() => handleBuiltinIcon(option.id)}
				>
					<NotebookGlyph icon={option.id} size={20} />
					<span>{option.label}</span>
				</button>
			{/each}
			<button type="button" class="icon-picker-more" aria-expanded={expanded} onclick={() => showAll = !expanded}>
				{expanded ? 'Fewer icons' : 'More icons'}
			</button>
		</div>
		<div class="color-section">
			<span class="color-label">Color</span>
			<div class="swatches">
				<button
					type="button"
					class="swatch none"
					class:active={!activeColor}
					aria-label="Use default color"
					title="Default color"
					onclick={() => persist({ kind: 'color', value: null })}
				></button>
				{#each TAG_COLOR_PRESETS as preset (preset)}
					<button
						type="button"
						class="swatch"
						class:active={activeColor?.toLowerCase() === preset}
						style:background={preset}
						aria-label={`Use ${preset}`}
						title={preset}
						onclick={() => persist({ kind: 'color', value: preset })}
					></button>
				{/each}
				<label class="swatch custom" title="Custom color">
					<input type="color" value={colorInputValue} oninput={onColorInput} aria-label="Custom tag color" />
				</label>
			</div>
		</div>
		<div class="icon-picker-actions">
			<button type="button" onclick={handleCustomIcon}>
				<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2" /><circle cx="8.5" cy="8.5" r="1.5" /><path d="m21 15-5-5L5 21" /></svg>
				Custom image...
			</button>
			{#if hasStyle}
				<button type="button" class="remove" onclick={handleReset}>
					<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><path d="M4 7h16M9 7V4h6v3M7 7l1 13h8l1-13" /></svg>
					Use default
				</button>
			{/if}
			<p class="icon-picker-recommendation">Icons and colors are optional. Recommended image size: <strong>1:1</strong></p>
		</div>
	</div>
</div>

<style>
	.picker-error {
		padding: 8px 16px;
		font-size: 13px;
		color: var(--text-primary);
		overflow-wrap: anywhere;
	}

	.icon-picker-overlay {
		position: fixed;
		inset: 0;
		z-index: 1100;
		display: grid;
		place-items: center;
		padding: 20px;
	}

	.icon-picker-backdrop {
		position: absolute;
		inset: 0;
		width: 100%;
		height: 100%;
		padding: 0;
		border: none;
		background: rgba(8, 10, 18, 0.42);
		cursor: default;
	}

	.icon-picker {
		position: relative;
		z-index: 1;
		width: min(360px, 100%);
		max-height: calc(100dvh - 40px);
		overflow-y: auto;
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 14px;
		box-shadow: var(--shadow-lg);
	}

	.icon-picker-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 16px;
		padding: 16px 18px 12px;
		border-bottom: 1px solid var(--border-light);
	}

	.icon-picker-header h3,
	.icon-picker-header p {
		margin: 0;
	}

	.icon-picker-header h3 {
		color: var(--text-primary);
		font-size: 15px;
		font-weight: 650;
	}

	.icon-picker-header p {
		margin-top: 6px;
		color: var(--text-tertiary);
		font-size: 12px;
	}

	.icon-picker-close {
		width: 32px;
		height: 32px;
		display: grid;
		place-items: center;
		padding: 0;
		border: none;
		border-radius: 8px;
		background: transparent;
		color: var(--text-tertiary);
		cursor: pointer;
	}

	.icon-picker-close:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.icon-picker-grid {
		display: grid;
		grid-template-columns: repeat(4, minmax(0, 1fr));
		gap: 7px;
		padding: 14px;
	}

	.icon-picker-option {
		min-width: 0;
		min-height: 60px;
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		gap: 5px;
		padding: 7px 3px 6px;
		border: 1px solid transparent;
		border-radius: 9px;
		background: var(--bg-secondary);
		color: var(--text-secondary);
		cursor: pointer;
	}

	.icon-picker-option span {
		max-width: 100%;
		overflow: hidden;
		color: var(--text-tertiary);
		font-size: 10px;
		line-height: 1.1;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.icon-picker-option:hover {
		border-color: var(--border-color);
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.icon-picker-option.active {
		border-color: var(--accent);
		background: var(--accent-light);
		color: var(--accent);
	}

	.icon-picker-option.active span {
		color: var(--accent);
	}

	.icon-picker-more {
		grid-column: 1 / -1;
		padding: 7px;
		border: none;
		border-radius: 8px;
		background: transparent;
		color: var(--text-tertiary);
		font-size: 12px;
		cursor: pointer;
	}

	.icon-picker-more:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.color-section {
		display: flex;
		flex-direction: column;
		gap: 8px;
		padding: 0 14px 12px;
	}

	.color-label {
		color: var(--text-secondary);
		font-size: 12px;
		font-weight: 600;
	}

	.swatches {
		display: flex;
		flex-wrap: wrap;
		gap: 7px;
		align-items: center;
	}

	.swatch {
		width: 22px;
		height: 22px;
		padding: 0;
		border: 2px solid transparent;
		border-radius: 50%;
		background: var(--bg-tertiary);
		cursor: pointer;
	}

	.swatch:hover,
	.swatch.active {
		border-color: var(--text-primary);
	}

	.swatch.none {
		background:
			linear-gradient(to bottom right, transparent calc(50% - 1px), var(--text-tertiary) calc(50% - 1px), var(--text-tertiary) calc(50% + 1px), transparent calc(50% + 1px)),
			var(--bg-secondary);
		border-color: var(--border-color);
	}

	.swatch.custom {
		display: grid;
		place-items: center;
		overflow: hidden;
		background: conic-gradient(from 90deg, #e11d48, #eab308, #22c55e, #3b82f6, #8b5cf6, #e11d48);
	}

	.swatch.custom input {
		width: 28px;
		height: 28px;
		padding: 0;
		border: none;
		background: none;
		cursor: pointer;
		transform: scale(1.4);
	}

	.icon-picker-actions {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
		padding: 12px 14px 14px;
		border-top: 1px solid var(--border-light);
	}

	.icon-picker-recommendation {
		flex: 0 0 100%;
		margin: 2px 0 0;
		color: var(--text-tertiary);
		font-size: 11px;
		line-height: 1.3;
		text-align: center;
	}

	.icon-picker-actions button {
		flex: 1;
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 7px;
		padding: 9px 12px;
		border: 1px solid var(--border-color);
		border-radius: 8px;
		background: var(--bg-secondary);
		color: var(--text-secondary);
		font-size: 12px;
		cursor: pointer;
	}

	.icon-picker-actions button:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.icon-picker-actions button.remove {
		color: var(--text-tertiary);
	}

	.icon-picker-overlay.mobile {
		padding: 12px;
		align-items: end;
	}

	.icon-picker-overlay.mobile .icon-picker {
		width: 100%;
		max-height: min(86dvh, 640px);
	}

	.icon-picker-overlay.mobile .icon-picker-option {
		min-height: 64px;
	}

	.icon-picker-overlay.mobile .icon-picker-more {
		min-height: 44px;
		font-size: 14px;
	}

	.icon-picker-overlay.mobile .icon-picker-actions button {
		min-height: 44px;
	}
</style>
