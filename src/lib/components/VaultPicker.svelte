<script lang="ts">
	import { onMount, untrack } from 'svelte';
	import { open } from '@tauri-apps/plugin-dialog';
	import { getCurrentWindow } from '@tauri-apps/api/window';
	import { documentDir } from '@tauri-apps/api/path';
	import { chooseExternalVault, completeWindowClose, getAppConfig, openVault, removeVault, restoreExternalVault } from '$lib/api';
	import { appConfig, vaultReady } from '$lib/stores/app';
	import { isAndroid, isIOS, isMobile } from '$lib/platform';
	import type { VaultConfig } from '$lib/types';

	let { initialError = '' }: { initialError?: string } = $props();

	const appWindow = getCurrentWindow();
	onMount(() => {
		let disposed = false;
		let unlisten: (() => void) | undefined;
		appWindow.onCloseRequested(async (event) => {
			event.preventDefault();
			try {
				await completeWindowClose();
			} catch (error) {
				console.error('Failed to close window:', error);
			}
		}).then((stop) => {
			if (disposed) stop();
			else unlisten = stop;
		});
		return () => { disposed = true; unlisten?.(); };
	});

	let recentVaults: VaultConfig[] = $derived($appConfig?.vaults ?? []);
	let loading = $state(false);
	let error = $state(untrack(() => initialError));
	let vaultName = $state('HelixNotes');
	let hasPermission = $state(!isAndroid);
	let selectedLocation = $state('Documents');

	let customPath = $state('');
	let iosBasePath = $state('');

	const storageLocations = [
		{ label: 'Documents', path: '/storage/emulated/0/Documents' },
		{ label: 'Downloads', path: '/storage/emulated/0/Download' },
		{ label: 'Internal Storage', path: '/storage/emulated/0' },
	];

	if (isAndroid) {
		checkPermission();
		setTimeout(checkPermission, 500);
		(window as any).__storagePermissionGranted = () => { hasPermission = true; };
		document.addEventListener('visibilitychange', () => {
			if (document.visibilityState === 'visible') setTimeout(checkPermission, 300);
		});
	}
	if (isIOS) {
		// iOS is sandboxed: the vault lives in the app's Documents dir (exposed via the Files app).
		documentDir().then((d) => { iosBasePath = d.replace(/\/+$/, ''); }).catch(() => {});
	}

	function checkPermission() {
		try {
			const bridge = (window as any).Android;
			if (bridge?.hasStoragePermission) {
				hasPermission = bridge.hasStoragePermission() === true;
			}
		} catch { /* bridge not ready */ }
	}

	function requestPermission() {
		try {
			const bridge = (window as any).Android;
			bridge?.requestStoragePermission?.();
		} catch {
			error = 'Go to Settings > Apps > HelixNotes > Permissions and enable "All files access".';
		}
	}

	function getMobileBasePath(): string {
		if (selectedLocation === 'Custom') {
			return customPath.trim() || '/storage/emulated/0/Documents';
		}
		return storageLocations.find(l => l.label === selectedLocation)?.path || '/storage/emulated/0/Documents';
	}

	let fullPath = $derived(
		isIOS ? `${iosBasePath}/${vaultName.trim() || 'HelixNotes'}`
		: isAndroid ? `${getMobileBasePath()}/${vaultName.trim() || 'HelixNotes'}`
		: ''
	);

	async function pickFolder() {
		if (isIOS) {
			const base = iosBasePath || (await documentDir()).replace(/\/+$/, '');
			await openSelectedVault(`${base}/${vaultName.trim() || 'HelixNotes'}`);
		} else if (isAndroid) {
			await openSelectedVault(fullPath);
		} else {
			const selected = await open({ directory: true, multiple: false, title: 'Choose Notes Folder' });
			if (selected) {
				await openSelectedVault(selected as string);
			}
		}
	}

	async function openSelectedVault(path: string) {
		loading = true;
		error = '';
		try {
			if (isAndroid) {
				try {
					const bridge = (window as any).Android;
					bridge?.prepareVaultDir?.(path);
				} catch { /* Rust will retry mkdir itself */ }
			}
			await openVault(path);
			const config = await getAppConfig();
			$appConfig = config;
			$vaultReady = true;
		} catch (e) {
			const msg = String(e);
			if (isAndroid && /os error 13|permission denied/i.test(msg)) {
				error = 'Storage permission is required. Enable file access for HelixNotes in your system settings, then try again.';
			} else {
				error = msg;
			}
		} finally {
			loading = false;
		}
	}

	async function pickExternalFolder() {
		loading = true;
		error = '';
		try {
			const selected = await chooseExternalVault();
			if (!selected) return;
			$appConfig = await getAppConfig();
			$vaultReady = true;
		} catch (e) {
			error = String(e);
		} finally {
			loading = false;
		}
	}

	async function openRecentVault(vault: VaultConfig) {
		if (loading) return;
		if (!isIOS || !vault.bookmark_id) {
			await openSelectedVault(vault.path);
			return;
		}

		loading = true;
		error = '';
		try {
			await restoreExternalVault(vault.bookmark_id);
			$appConfig = await getAppConfig();
			$vaultReady = true;
		} catch (e) {
			error = String(e);
		} finally {
			loading = false;
		}
	}

	async function forgetVault(vault: VaultConfig) {
		if (loading) return;
		loading = true;
		error = '';
		try {
			const wasActive = isActiveVault(vault);
			await removeVault(vault.path, vault.bookmark_id);
			$appConfig = await getAppConfig();
			if (wasActive) $vaultReady = false;
		} catch (e) {
			error = String(e);
		} finally {
			loading = false;
		}
	}

	function isActiveVault(vault: VaultConfig): boolean {
		if (vault.bookmark_id) {
			return vault.bookmark_id === $appConfig?.active_bookmark_id;
		}
		return !$appConfig?.active_bookmark_id && vault.path === $appConfig?.active_vault;
	}
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="vault-picker" class:mobile={isMobile} onmousedown={(e) => { if (!isMobile && !(e.target as HTMLElement).closest('button, .picker-card')) appWindow.startDragging(); }}>
	{#if !isMobile}
	<div class="window-controls">
		<button class="window-close" onclick={() => appWindow.close()} title="Close">
			<svg width="10" height="10" viewBox="0 0 10 10">
				<line x1="1.5" y1="1.5" x2="8.5" y2="8.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
				<line x1="8.5" y1="1.5" x2="1.5" y2="8.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
			</svg>
		</button>
	</div>
	{/if}
	<div class="picker-card">
		<div class="logo">
			<svg width="72" height="72" viewBox="0 0 48 48" fill="none">
				<rect width="48" height="48" rx="12" fill="var(--accent)" />
				<circle cx="16" cy="16" r="3.5" fill="white" opacity="0.9" />
				<circle cx="32" cy="16" r="3.5" fill="white" opacity="0.9" />
				<circle cx="16" cy="32" r="3.5" fill="white" opacity="0.9" />
				<circle cx="32" cy="32" r="3.5" fill="white" opacity="0.9" />
				<line x1="19" y1="18" x2="29" y2="30" stroke="white" stroke-width="2" stroke-linecap="round" opacity="0.7" />
				<line x1="29" y1="18" x2="19" y2="30" stroke="white" stroke-width="2" stroke-linecap="round" opacity="0.7" />
			</svg>
		</div>
		<h1>HelixNotes</h1>
		<p class="subtitle">Local markdown notes</p>
		{#if isMobile}
			{#if isIOS}
				<p class="description">Keep notes in HelixNotes, or use an existing folder from iCloud Drive or Files.</p>
			{:else}
				<p class="description">Choose where to store your notes. Sync with Syncthing, Nextcloud, or any file sync app.</p>
			{/if}

			{#if isAndroid && !hasPermission}
				<p class="permission-hint">
					HelixNotes needs "All files access" to read and write notes in Documents. Tap the button below,
					then turn on All files access for HelixNotes.
				</p>
			{/if}

			{#if isAndroid}
			<div class="location-selector" role="group" aria-labelledby="storage-location-label">
				<span id="storage-location-label" class="location-label">Location</span>
				<div class="location-options">
					{#each storageLocations as loc}
						<button
							class="location-option"
							class:active={selectedLocation === loc.label}
							aria-pressed={selectedLocation === loc.label}
							onclick={() => selectedLocation = loc.label}
						>{loc.label}</button>
					{/each}
					<button
						class="location-option"
						class:active={selectedLocation === 'Custom'}
						aria-pressed={selectedLocation === 'Custom'}
						onclick={() => selectedLocation = 'Custom'}
					>Custom</button>
				</div>
				{#if selectedLocation === 'Custom'}
					<input class="custom-path-input" type="text" bind:value={customPath} placeholder="/storage/emulated/0/..." />
				{/if}
			</div>
			{/if}

			<div class="vault-name-input">
				<label for="vault-name">Vault name</label>
				<div class="vault-name-preview"><input id="vault-name" type="text" bind:value={vaultName} placeholder="HelixNotes" /></div>
				<span class="vault-path-hint">{fullPath}</span>
			</div>
		{:else}
			<p class="description">Your notes are stored as standard Markdown (.md) files. Pick any folder - existing .md files will be recognized automatically.</p>
		{/if}

		{#if error}
			<div class="error">{error}</div>
		{/if}

		<button class="btn-primary" onclick={isAndroid && !hasPermission ? requestPermission : pickFolder} disabled={loading}>
			{#if loading}
				Opening...
			{:else if isAndroid && !hasPermission}
				Grant File Access
			{:else if isIOS}
				Use Local Folder
			{:else if isMobile}
				Get Started
			{:else}
				Open Notes Folder
			{/if}
		</button>

		{#if isIOS}
			<button class="btn-back" onclick={pickExternalFolder} disabled={loading}>
				Choose Folder in Files
			</button>
		{/if}

		{#if $appConfig?.active_vault && !initialError}
			<button class="btn-back" onclick={() => ($vaultReady = true)}>
				Back
			</button>
		{/if}

		{#if recentVaults.length > 0}
			<div class="recent">
				<span class="recent-label">Recent</span>
				{#each recentVaults as vault}
					<div class="vault-row">
						<button class="vault-item" class:current={isActiveVault(vault)} onclick={() => openRecentVault(vault)} disabled={loading}>
							<span class="vault-name">{vault.name}{#if isActiveVault(vault)}<span class="vault-current-badge">Current</span>{/if}</span>
							<span class="vault-path">{vault.bookmark_id ? `Files · ${vault.path}` : vault.path}</span>
						</button>
						<button class="vault-remove" title="Remove from list" aria-label="Remove from list" onclick={() => forgetVault(vault)} disabled={loading}>
							<svg width="12" height="12" viewBox="0 0 10 10">
								<line x1="1.5" y1="1.5" x2="8.5" y2="8.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" />
								<line x1="8.5" y1="1.5" x2="1.5" y2="8.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" />
							</svg>
						</button>
					</div>
				{/each}
			</div>
		{/if}
	</div>
</div>

<style>
	.vault-picker {
		display: flex;
		justify-content: center;
		height: 100vh;
		height: 100dvh;
		background: var(--bg-primary);
		position: relative;
		overflow-y: auto;
		-webkit-overflow-scrolling: touch;
	}

	.window-controls {
		position: absolute;
		top: 8px;
		right: 8px;
		z-index: 10;
	}

	.window-close {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 32px;
		height: 28px;
		border: none;
		background: none;
		color: var(--text-tertiary);
		cursor: pointer;
		border-radius: 6px;
		transition: background 0.1s, color 0.1s;
	}

	.window-close:hover {
		background: #e81123;
		color: white;
	}

	.picker-card {
		text-align: center;
		max-width: 400px;
		width: 100%;
		padding: 48px 32px;
		padding-bottom: calc(120px + env(safe-area-inset-bottom, 0px));
		margin: auto;
		flex-shrink: 0;
	}

	.logo {
		margin-bottom: 16px;
		display: flex;
		justify-content: center;
	}

	h1 {
		font-size: 24px;
		font-weight: 700;
		margin-bottom: 4px;
		color: var(--text-primary);
	}

	.subtitle {
		color: var(--text-tertiary);
		margin-bottom: 8px;
		font-size: 14px;
	}

	.description {
		color: var(--text-tertiary);
		font-size: 12px;
		line-height: 1.5;
		margin-bottom: 24px;
		opacity: 0.8;
	}

	.error {
		background: color-mix(in srgb, var(--danger) 10%, transparent);
		color: var(--danger);
		padding: 8px 12px;
		border-radius: 6px;
		margin-bottom: 16px;
		font-size: 13px;
	}

	.btn-primary {
		width: 100%;
		padding: 10px 20px;
		background: var(--accent);
		color: white;
		border: none;
		border-radius: 8px;
		font-size: 14px;
		font-weight: 500;
		cursor: pointer;
		transition: background 0.15s;
	}

	.btn-primary:hover {
		background: var(--accent-hover);
	}

	.btn-primary:disabled {
		opacity: 0.6;
		cursor: not-allowed;
	}

	.btn-back {
		width: 100%;
		padding: 10px 20px;
		background: var(--bg-tertiary);
		color: var(--text-secondary);
		border: 1px solid var(--border-color);
		border-radius: 8px;
		font-size: 14px;
		font-weight: 500;
		cursor: pointer;
		margin-top: 8px;
		transition: background 0.15s, color 0.15s;
	}

	.btn-back:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.recent {
		margin-top: 32px;
		text-align: left;
	}

	.recent-label {
		font-size: 11px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--text-tertiary);
		display: block;
		margin-bottom: 8px;
	}

	.vault-item {
		display: block;
		width: 100%;
		padding: 10px 12px;
		background: var(--bg-secondary);
		border: 1px solid var(--border-color);
		border-radius: 8px;
		cursor: pointer;
		text-align: left;
		margin-bottom: 6px;
		transition: background 0.15s;
	}

	.vault-item:hover {
		background: var(--bg-hover);
	}

	.vault-item.current {
		border-color: var(--accent);
	}

	.vault-current-badge {
		display: inline-block;
		font-size: 10px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.04em;
		color: var(--accent);
		background: color-mix(in srgb, var(--accent) 12%, transparent);
		padding: 2px 7px;
		border-radius: 10px;
		margin-left: 8px;
		vertical-align: middle;
	}

	.vault-name {
		display: block;
		font-weight: 500;
		font-size: 14px;
		color: var(--text-primary);
	}

	.vault-path {
		display: block;
		font-size: 12px;
		color: var(--text-tertiary);
		margin-top: 2px;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.vault-row {
		display: flex;
		align-items: stretch;
		gap: 6px;
		margin-bottom: 6px;
	}

	.vault-row .vault-item {
		flex: 1;
		min-width: 0;
		margin-bottom: 0;
	}

	.vault-remove {
		display: flex;
		align-items: center;
		justify-content: center;
		flex-shrink: 0;
		width: 36px;
		background: var(--bg-secondary);
		border: 1px solid var(--border-color);
		border-radius: 8px;
		color: var(--text-tertiary);
		cursor: pointer;
		opacity: 0;
		transition: opacity 0.15s, background 0.15s, color 0.15s;
	}

	.vault-row:hover .vault-remove,
	.vault-remove:focus-visible {
		opacity: 1;
	}

	.vault-remove:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.vault-picker.mobile .vault-remove {
		opacity: 1;
	}

	.vault-name-input {
		margin-bottom: 20px;
		text-align: left;
	}

	.vault-name-input label {
		display: block;
		font-size: 12px;
		font-weight: 500;
		color: var(--text-tertiary);
		margin-bottom: 6px;
	}

	.vault-name-preview {
		display: flex;
		align-items: center;
		background: var(--bg-secondary);
		border: 1px solid var(--border-color);
		border-radius: 8px;
		padding: 10px 12px;
		font-size: 14px;
		color: var(--text-tertiary);
	}

	.vault-name-preview input {
		background: none;
		border: none;
		outline: none;
		color: var(--text-primary);
		font-size: 14px;
		font-family: inherit;
		width: 100%;
		padding: 0;
	}

	.location-selector {
		margin-bottom: 16px;
		text-align: left;
	}

	.location-label {
		display: block;
		font-size: 12px;
		font-weight: 500;
		color: var(--text-tertiary);
		margin-bottom: 6px;
	}

	.location-options {
		display: flex;
		gap: 6px;
		flex-wrap: wrap;
	}

	.location-option {
		padding: 8px 14px;
		border: 1px solid var(--border-color);
		border-radius: 8px;
		background: var(--bg-secondary);
		color: var(--text-secondary);
		font-size: 13px;
		font-family: inherit;
		cursor: pointer;
		transition: all 0.15s;
	}

	.location-option.active {
		background: var(--accent);
		color: white;
		border-color: var(--accent);
	}

	.custom-path-input {
		width: 100%;
		margin-top: 8px;
		padding: 10px 12px;
		background: var(--bg-secondary);
		border: 1px solid var(--border-color);
		border-radius: 8px;
		color: var(--text-primary);
		font-size: 13px;
		font-family: inherit;
		outline: none;
		box-sizing: border-box;
	}

	.custom-path-input:focus {
		border-color: var(--accent);
	}

	.vault-path-hint {
		display: block;
		font-size: 11px;
		color: var(--text-tertiary);
		margin-top: 6px;
		opacity: 0.7;
		word-break: break-all;
	}

	.permission-hint {
		font-size: 12px;
		color: var(--text-tertiary);
		margin-bottom: 20px;
		opacity: 0.7;
	}

	/* Mobile */
	.mobile .btn-primary {
		padding: 14px 20px;
		font-size: 16px;
		border-radius: 12px;
	}

	.mobile .description {
		font-size: 14px;
	}

	.mobile h1 {
		font-size: 28px;
	}

	.mobile .logo :global(svg) {
		width: 88px;
		height: 88px;
	}

	.mobile .btn-back {
		padding: 14px 20px;
		font-size: 16px;
		border-radius: 12px;
	}

	.mobile .vault-item {
		padding: 14px 16px;
	}

	.mobile .vault-name {
		font-size: 15px;
	}

	.mobile .vault-path {
		font-size: 13px;
	}
</style>
