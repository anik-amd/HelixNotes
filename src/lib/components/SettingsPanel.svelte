<script lang="ts">
	import { untrack } from 'svelte';
	import { showSettings, syncState, theme, resolvedTheme, appConfig, platformIsMobile, activeVaultConfig, updateAvailable as globalUpdateAvailable, updateObj as globalUpdateObj, installType, settingsTab, vaultReady, androidApkUrl, checkForUpdateMobile, notebookSortMode, isManagedInstall, customThemes } from '$lib/stores/app';
	import { setTheme, setSystemThemes, setAccentColor, setFontSize, setScrollToChangeFontSize, setFontFamily, setLineHeight, setUiScale, setContentWidth, setGeneralSettings, importObsidian, createBackup, listBackups, restoreBackup, deleteBackup, setBackupSettings, setAiSettings, testAiConnection, setSyncSettings, testSyncConnection, syncNow, saveCustomTheme, deleteCustomTheme, exportCustomTheme, importCustomThemes, getVaultStats, findOrphanedAttachments, trashOrphanedAttachments } from '$lib/api';
	import { darkThemes, isMobile, isAndroid } from '$lib/platform';
	import { open as openDialog, save as saveDialog } from '@tauri-apps/plugin-dialog';
	import { listen } from '@tauri-apps/api/event';
	import { getVersion } from '@tauri-apps/api/app';
	import { applyUiScale, defaultUiScale, uiScalePresets } from '$lib/ui-scale';
	import { openUrl } from '$lib/api';
	import type { OrphanAttachment } from '$lib/api';
	import type { ImportResult, BackupEntry, CustomTheme, CustomThemeColors, StartupView, VaultStats } from '$lib/types';
	import { normalizeStartupView } from '$lib/utils/startup-view';

	let { onBeforeVaultSwitch }: { onBeforeVaultSwitch: () => Promise<boolean> } = $props();

	const modKey = navigator.platform.startsWith('Mac') ? '⌘' : 'Ctrl';

	type Tab = 'general' | 'editor' | 'styling' | 'import' | 'backup' | 'maintenance' | 'ai' | 'sync' | 'updates';
	let activeTab = $state<Tab>('styling');

	// Updates state
	let updateChecking = $state(false);
	let updateAvailable = $state<{ version: string; body?: string; date?: string } | null>(null);
	let updateDownloading = $state(false);
	let updateProgress = $state(0);
	let updateMessage = $state<{ type: 'success' | 'error' | 'info'; text: string } | null>(null);
	let updateObj = $state<any>(null);
	let appVersion = $state('...');

	async function loadAppVersion() {
		try { appVersion = await getVersion(); } catch { appVersion = '0.0.0'; }
	}
	loadAppVersion();

	// Switch to requested tab if set externally (e.g. from update badge)
	$effect(() => {
		const tab = $settingsTab;
		if (tab) {
			activeTab = tab as Tab;
			$settingsTab = null;
		}
	});

	// Pre-populate from global store if update was already detected at startup
	$effect(() => {
		const global = $globalUpdateAvailable;
		if (global && !updateAvailable) {
			updateAvailable = { version: global.version, body: global.body };
		}
	});

	// Pre-populate updater object from global store so Download & Install works without manual check
	$effect(() => {
		const obj = $globalUpdateObj;
		if (obj && !updateObj) {
			updateObj = obj;
		}
	});

	async function handleCheckUpdate() {
		updateChecking = true;
		updateMessage = null;
		updateAvailable = null;
		try {
			if (isMobile) {
				await checkForUpdateMobile();
				const global = $globalUpdateAvailable;
				if (global) {
					updateAvailable = { version: global.version, body: global.body };
					updateMessage = { type: 'info', text: `Version ${global.version} is available!` };
				} else {
					updateMessage = { type: 'success', text: 'You are on the latest version.' };
				}
			} else {
				const { check: checkUpdate } = await import('@tauri-apps/plugin-updater');
				const update = await checkUpdate();
				if (update) {
					updateObj = update;
					$globalUpdateObj = update;
					updateAvailable = { version: update.version, body: update.body, date: update.date };
					globalUpdateAvailable.set({ version: update.version, body: update.body });
					updateMessage = { type: 'info', text: `Version ${update.version} is available!` };
				} else {
					updateMessage = { type: 'success', text: 'You are on the latest version.' };
				}
			}
		} catch (e) {
			updateMessage = { type: 'error', text: `Failed to check: ${e}` };
		} finally {
			updateChecking = false;
		}
	}

	async function handleDownloadAndInstall() {
		if (!updateObj) return;
		updateDownloading = true;
		updateProgress = 0;
		updateMessage = { type: 'info', text: 'Downloading update...' };
		try {
			let totalBytes = 0;
			let downloadedBytes = 0;
			await updateObj.downloadAndInstall((event: any) => {
				if (event.event === 'Started' && event.data.contentLength) {
					totalBytes = event.data.contentLength;
				} else if (event.event === 'Progress') {
					downloadedBytes += event.data.chunkLength;
					if (totalBytes > 0) {
						updateProgress = Math.round((downloadedBytes / totalBytes) * 100);
					}
				} else if (event.event === 'Finished') {
					updateProgress = 100;
				}
			});
			updateMessage = { type: 'success', text: 'Update installed! Restart the app to apply.' };
		} catch (e) {
			updateMessage = { type: 'error', text: `Update failed: ${e}` };
		} finally {
			updateDownloading = false;
		}
	}

	// Vault maintenance state
	let vaultStats = $state<VaultStats | null>(null);
	let vaultStatsError = $state(false);
	let orphanedAttachments = $state<OrphanAttachment[] | null>(null);
	let scanningOrphanedAttachments = $state(false);
	let trashingOrphanedAttachments = $state(false);
	let orphanedVault: string | null = null;
	let orphanError = $state('');
	let cleanupRequest = 0;
	const maintenanceVault = $derived($appConfig?.active_vault ?? null);
	const orphanedAttachmentTotal = $derived((orphanedAttachments ?? []).reduce((total, attachment) => total + attachment.size, 0));

	$effect(() => {
		void maintenanceVault;
		cleanupRequest++;
		orphanedAttachments = null;
		orphanedVault = null;
		orphanError = '';
		scanningOrphanedAttachments = false;
		trashingOrphanedAttachments = false;
		if (!$showSettings) {
			vaultStats = null;
			vaultStatsError = false;
			orphanedAttachments = null;
			return;
		}
		if (activeTab === 'maintenance') void loadVaultStats();
	});

	async function loadVaultStats() {
		const vault = maintenanceVault;
		vaultStatsError = false;
		try {
			const stats = await getVaultStats();
			if ($showSettings && maintenanceVault === vault) vaultStats = stats;
		} catch (error) {
			if (!$showSettings || maintenanceVault !== vault) return;
			vaultStats = null;
			vaultStatsError = true;
			console.error('Failed to load vault statistics:', error);
		}
	}

	function formatVaultSize(bytes: number): string {
		if (bytes < 1024) return `${bytes} B`;
		if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
		if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
		return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
	}

	async function scanOrphanedAttachments() {
		const vault = maintenanceVault;
		if (!vault) return;
		const request = ++cleanupRequest;
		const current = () => request === cleanupRequest && $showSettings && maintenanceVault === vault;
		scanningOrphanedAttachments = true;
		orphanError = '';
		try {
			if (!(await onBeforeVaultSwitch())) throw new Error('Save the current note before scanning attachments.');
			if (!current()) return;
			const found = await findOrphanedAttachments(vault);
			if (!current()) return;
			orphanedVault = vault;
			orphanedAttachments = found;
		} catch (error) {
			if (current()) orphanError = String(error);
		} finally {
			if (current()) scanningOrphanedAttachments = false;
		}
	}

	async function trashFoundOrphanedAttachments() {
		const vault = orphanedVault;
		if (!vault || vault !== maintenanceVault || !orphanedAttachments?.length) return;
		const names = orphanedAttachments.map(attachment => attachment.name);
		const request = ++cleanupRequest;
		const current = () => request === cleanupRequest && $showSettings && maintenanceVault === vault;
		trashingOrphanedAttachments = true;
		orphanError = '';
		try {
			if (!(await onBeforeVaultSwitch())) throw new Error('Save the current note before cleaning attachments.');
			if (!current()) return;
			await trashOrphanedAttachments(vault, names);
			if (!current()) return;
			orphanedAttachments = null;
			orphanedVault = null;
			void loadVaultStats();
		} catch (error) {
			if (current()) orphanError = String(error);
		} finally {
			if (current()) trashingOrphanedAttachments = false;
		}
	}

	// Backup state
	let backups = $state<BackupEntry[]>([]);
	let backupLoading = $state(false);
	let backupMessage = $state<{ type: 'success' | 'error'; text: string } | null>(null);
	let restoreConfirm = $state<BackupEntry | null>(null);
	let backupRequest = 0;
	let backupReadRequest = 0;
	let backupUnlisten: (() => void) | null = null;
	const lastBackupTime = $derived(activeVaultConfig($appConfig)?.last_backup_time ?? null);

	$effect(() => {
		void maintenanceVault;
		void $showSettings;
		backupRequest++;
		backupReadRequest++;
		backupUnlisten?.();
		backupUnlisten = null;
		backupLoading = false;
		return () => {
			backupRequest++;
			backupReadRequest++;
			backupUnlisten?.();
			backupUnlisten = null;
		};
	});

	async function loadBackups() {
		const vault = maintenanceVault;
		if (!vault) return;
		const request = ++backupReadRequest;
		try {
			const loaded = await listBackups(vault);
			if (request === backupReadRequest && maintenanceVault === vault && $showSettings) backups = loaded;
		} catch (e) {
			console.error('Failed to load backups:', e);
		}
	}

	async function handleBackupNow() {
		const vault = maintenanceVault;
		if (!vault || backupLoading) return;
		const request = ++backupRequest;
		const current = () => request === backupRequest && maintenanceVault === vault && $showSettings;
		backupLoading = true;
		backupMessage = null;
		let stop: (() => void) | undefined;
		let finished = false;
		try {
			if (!(await onBeforeVaultSwitch())) throw new Error('Save the current note before creating a backup.');
			if (!current()) return;
			stop = await listen<{ vault_path: string; success: boolean; entry?: BackupEntry; error?: string }>('backup-done', (event) => {
				const data = event.payload;
				if (data.vault_path !== vault || !current()) return;
				finished = true;
				stop?.();
				backupUnlisten = null;
				backupLoading = false;
				if (data.success) {
					backupMessage = { type: 'success', text: 'Backup created successfully' };
					const timestamp = data.entry?.created;
					const currentVault = activeVaultConfig($appConfig);
					if (timestamp && currentVault && $appConfig) {
						$appConfig = { ...$appConfig, vaults: $appConfig.vaults.map(item =>
							item === currentVault ? { ...item, last_backup_time: timestamp } : item) };
					}
					void loadBackups();
				} else {
					backupMessage = { type: 'error', text: `Backup failed: ${data.error}` };
				}
			});
			if (!current() || finished) { stop(); return; }
			backupUnlisten = stop;
			await createBackup(vault);
		} catch (e) {
			stop?.();
			if (current()) {
				backupUnlisten = null;
				backupMessage = { type: 'error', text: `Backup failed: ${e}` };
				backupLoading = false;
			}
		}
	}

	async function handleRestore(entry: BackupEntry) {
		const vault = maintenanceVault;
		if (!vault || backupLoading) return;
		const request = ++backupRequest;
		const current = () => request === backupRequest && maintenanceVault === vault && $showSettings;
		restoreConfirm = null;
		backupLoading = true;
		backupMessage = null;
		let stop: (() => void) | undefined;
		let finished = false;
		try {
			if (!(await onBeforeVaultSwitch())) throw new Error('Save the current note before restoring a backup.');
			if (!current()) return;
			stop = await listen<{ vault_path: string; success: boolean; error?: string }>('restore-done', (event) => {
				const data = event.payload;
				if (data.vault_path !== vault || !current()) return;
				finished = true;
				stop?.();
				backupUnlisten = null;
				backupMessage = data.success
					? { type: 'success', text: 'Backup restored successfully.' }
					: { type: 'error', text: `Restore failed: ${data.error}` };
				backupLoading = false;
			});
			if (!current() || finished) { stop(); return; }
			backupUnlisten = stop;
			await restoreBackup(vault, entry.path);
		} catch (e) {
			stop?.();
			if (current()) {
				backupUnlisten = null;
				backupMessage = { type: 'error', text: `Restore failed: ${e}` };
				backupLoading = false;
			}
		}
	}

	async function handleDeleteBackup(entry: BackupEntry) {
		const vault = maintenanceVault;
		if (!vault) return;
		try {
			await deleteBackup(vault, entry.path);
			if (maintenanceVault === vault) backups = backups.filter(b => b.path !== entry.path);
		} catch (e) {
			console.error('Failed to delete backup:', e);
		}
	}

	async function handleSelectBackupFolder() {
		const selected = await openDialog({ directory: true, multiple: false, title: 'Select backup folder' });
		if (selected && $appConfig) {
			await setBackupSettings($appConfig.backup_enabled, $appConfig.backup_frequency, $appConfig.backup_max_count, selected as string, $appConfig.backup_include_attachments);
			$appConfig = { ...$appConfig, backup_location: selected as string };
			await loadBackups();
		}
	}

	async function saveBackupSettings() {
		if (!$appConfig) return;
		await setBackupSettings($appConfig.backup_enabled, $appConfig.backup_frequency, $appConfig.backup_max_count, $appConfig.backup_location, $appConfig.backup_include_attachments);
	}

	function formatBackupSize(bytes: number): string {
		if (bytes < 1024) return `${bytes} B`;
		if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
		return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
	}

	function formatBackupDate(iso: string): string {
		try {
			const d = new Date(iso);
			return d.toLocaleString();
		} catch {
			return iso;
		}
	}

	// AI state
	let aiProvider = $state<string | null>($appConfig?.ai_provider ?? null);
	let aiApiKey = $derived.by(() => {
		if (aiProvider === 'openai') return _openaiKey;
		if (aiProvider === 'ollama') return '';
		if (aiProvider === 'openai_compatible') return _openaiCompatibleKey;
		return _anthropicKey;
	});
	let _anthropicKey = $state($appConfig?.ai_api_key ?? '');
	let _openaiKey = $state($appConfig?.openai_api_key ?? '');
	let _ollamaBaseUrl = $state($appConfig?.ollama_base_url ?? 'http://localhost:11434');
	let _ollamaApiKey = $state($appConfig?.ollama_api_key ?? '');
	let _openaiCompatibleBaseUrl = $state($appConfig?.openai_compatible_base_url ?? '');
	let _openaiCompatibleKey = $state($appConfig?.openai_compatible_api_key ?? '');
	let aiModel = $state($appConfig?.ai_model ?? 'claude-sonnet-5');
	let aiWritingStyle = $state($appConfig?.ai_writing_style ?? '');
	let aiShowKey = $state(false);
	let aiShowCompatibleKey = $state(false);
	let aiShowOllamaKey = $state(false);
	let aiTestLoading = $state(false);
	let aiTestMessage = $state<{ type: 'success' | 'error'; text: string } | null>(null);

	// General-purpose text models verified against provider catalogs on 2026-09-27.
	const anthropicModels = [
		{ value: 'claude-sonnet-5', label: 'Claude Sonnet 5', desc: 'Balanced' },
		{ value: 'claude-opus-5-5', label: 'Claude Opus 5.5', desc: 'Complex work' },
		{ value: 'claude-fable-5-1', label: 'Claude Fable 5.1', desc: 'Deep reasoning' },
		{ value: 'claude-haiku-4-5-20251001', label: 'Claude Haiku 4.5', desc: 'Fast' },
	];
	const openaiModels = [
		{ value: 'gpt-6-sol', label: 'GPT-6 Sol', desc: 'Balanced' },
		{ value: 'gpt-6-astra', label: 'GPT-6 Astra', desc: 'Deep reasoning' },
		{ value: 'gpt-6-luna', label: 'GPT-6 Luna', desc: 'Fast' },
	];
	let aiModels = $derived(aiProvider === 'openai' ? openaiModels : anthropicModels);

	async function saveAiSettings() {
		const baseUrl = aiProvider === 'ollama' ? (_ollamaBaseUrl || null) : null;
		await setAiSettings(
			aiProvider,
			aiApiKey || null,
			aiModel,
			aiWritingStyle || null,
			baseUrl,
			_ollamaApiKey || null,
			_openaiCompatibleBaseUrl || null,
			_openaiCompatibleKey || null,
		);
		if ($appConfig) {
			$appConfig = {
				...$appConfig,
				ai_provider: aiProvider,
				ai_api_key: _anthropicKey || null,
				openai_api_key: _openaiKey || null,
				ollama_base_url: _ollamaBaseUrl || null,
				ollama_api_key: _ollamaApiKey || null,
				openai_compatible_base_url: _openaiCompatibleBaseUrl || null,
				openai_compatible_api_key: _openaiCompatibleKey || null,
				ai_model: aiModel,
				ai_writing_style: aiWritingStyle || null,
			};
		}
	}

	async function handleTestAi() {
		aiTestLoading = true;
		aiTestMessage = null;
		const unlisten = await listen<{ success: boolean; message?: string; error?: string }>('ai-test-result', (event) => {
			const data = event.payload;
			if (data.success) {
				aiTestMessage = { type: 'success', text: 'Connection successful!' };
			} else {
				aiTestMessage = { type: 'error', text: data.error ?? 'Connection failed' };
			}
			aiTestLoading = false;
			unlisten();
		});
		try {
			await testAiConnection();
		} catch (e) {
			aiTestMessage = { type: 'error', text: String(e) };
			aiTestLoading = false;
			unlisten();
		}
	}

	// ── WebDAV sync ──
	let syncProvider = $state<string | null>(activeVaultConfig($appConfig)?.sync_provider ?? null);
	let syncUrl = $state(activeVaultConfig($appConfig)?.webdav_url ?? '');
	let syncUsername = $state(activeVaultConfig($appConfig)?.webdav_username ?? '');
	let syncPassword = $state(activeVaultConfig($appConfig)?.webdav_password ?? '');
	let syncShowPassword = $state(false);
	let syncTestLoading = $state(false);
	let syncTestMessage = $state<{ type: 'success' | 'error'; text: string } | null>(null);
	let syncStarting = $state(false);
	const syncRunning = $derived(syncStarting || $syncState.running);
	let syncMessage = $state<{ type: 'success' | 'error'; text: string } | null>(null);
	let syncOnOpen = $state(activeVaultConfig($appConfig)?.sync_on_open ?? false);
	let syncOnChange = $state(activeVaultConfig($appConfig)?.sync_on_change ?? false);
	let syncIntervalMinutes = $state(activeVaultConfig($appConfig)?.sync_interval_minutes ?? 0);
	// 0 means never chosen, which the sync treats as DEFAULT_LISTING_PARALLELISM (6) in sync.rs.
	const defaultFolderRequests = 6;
	let syncFolderRequests = $state(activeVaultConfig($appConfig)?.sync_folder_requests ?? 0);

	// Re-seed the sync form when the active vault changes, so Settings reflects the current vault.
	let lastSyncVault: string | null = null;
	$effect(() => {
		const identity = $appConfig?.active_bookmark_id
			? `bookmark:${$appConfig.active_bookmark_id}`
			: $appConfig?.active_vault
				? `path:${$appConfig.active_vault}`
				: null;
		if (identity === lastSyncVault) return;
		lastSyncVault = identity;
		const vc = activeVaultConfig($appConfig);
		syncProvider = vc?.sync_provider ?? null;
		syncUrl = vc?.webdav_url ?? '';
		syncUsername = vc?.webdav_username ?? '';
		syncPassword = vc?.webdav_password ?? '';
		syncOnOpen = vc?.sync_on_open ?? false;
		syncOnChange = vc?.sync_on_change ?? false;
		syncIntervalMinutes = vc?.sync_interval_minutes ?? 0;
		syncFolderRequests = vc?.sync_folder_requests ?? 0;
	});

	async function saveSyncSettings() {
		await setSyncSettings(syncProvider, syncUrl || null, syncUsername || null, syncPassword || null, syncOnOpen, syncOnChange, syncIntervalMinutes, syncFolderRequests);
		// Reflect into the active vault's config in the store so the top-bar button and auto-sync
		// triggers update live, without an app restart.
		if ($appConfig) {
			const cur = $appConfig;
			const active = activeVaultConfig(cur);
			$appConfig = {
				...cur,
				vaults: cur.vaults.map((vault) => vault === active ? {
					...vault,
					sync_provider: syncProvider,
					webdav_url: syncUrl || null,
					webdav_username: syncUsername || null,
					webdav_password: syncPassword || null,
					sync_on_open: syncOnOpen,
					sync_on_change: syncOnChange,
					sync_interval_minutes: syncIntervalMinutes,
					sync_folder_requests: syncFolderRequests,
				} : vault),
			};
		}
	}

	async function handleTestSync() {
		syncTestLoading = true;
		syncTestMessage = null;
		const unlisten = await listen<{ success: boolean; message?: string; error?: string }>('sync-test-result', (event) => {
			const data = event.payload;
			syncTestMessage = data.success
				? { type: 'success', text: data.message ?? 'Connection successful' }
				: { type: 'error', text: data.error ?? 'Connection failed' };
			syncTestLoading = false;
			unlisten();
		});
		try {
			await saveSyncSettings();
			await testSyncConnection();
		} catch (e) {
			syncTestMessage = { type: 'error', text: String(e) };
			syncTestLoading = false;
			unlisten();
		}
	}

	// The outcome of the last sync, whether it was started here, from the title bar or automatically.
	$effect(() => {
		const state = $syncState;
		const vault = maintenanceVault;
		untrack(() => {
			if (state.running) syncMessage = null;
			else if (state.error) syncMessage = { type: 'error', text: state.error };
			else if (state.result && state.result.vault === vault) {
				const s = state.result.summary;
				const parts: string[] = [];
				if (s.uploaded) parts.push(`${s.uploaded} uploaded`);
				if (s.downloaded) parts.push(`${s.downloaded} downloaded`);
				const deleted = (s.deleted_local ?? 0) + (s.deleted_remote ?? 0);
				if (deleted) parts.push(`${deleted} deleted`);
				if (s.conflicts) parts.push(`${s.conflicts} conflict ${s.conflicts === 1 ? 'copy' : 'copies'}`);
				syncMessage = { type: 'success', text: parts.length ? `Synced: ${parts.join(', ')}.` : 'Already up to date.' };
			}
		});
	});

	async function handleSyncNow() {
		if (!maintenanceVault || syncRunning) return;
		syncStarting = true;
		syncMessage = null;
		try {
			await saveSyncSettings();
			await syncNow();
		} catch (e) {
			syncMessage = { type: 'error', text: String(e) };
		} finally {
			syncStarting = false;
		}
	}

	let isThemeDark = $derived(
		darkThemes.includes($resolvedTheme) ||
		($resolvedTheme.startsWith('custom-') && ($customThemes.find(c => c.id === $resolvedTheme)?.is_dark ?? false))
	);

	const themePresets = [
		{ id: 'system', label: 'System', bg: '#ffffff', sidebar: '#1a1b26', accent: '#5b6abf' },
		{ id: 'light', label: 'Light', bg: '#ffffff', sidebar: '#f8f9fa', accent: '#5b6abf' },
		{ id: 'dark', label: 'Dark', bg: '#1a1b26', sidebar: '#1f2029', accent: '#89b4fa' },
		{ id: 'solarized-light', label: 'Solarized Light', bg: '#fdf6e3', sidebar: '#eee8d5', accent: '#268bd2' },
		{ id: 'solarized-dark', label: 'Solarized Dark', bg: '#002b36', sidebar: '#073642', accent: '#268bd2' },
		{ id: 'catppuccin', label: 'Catppuccin', bg: '#1e1e2e', sidebar: '#181825', accent: '#89b4fa' },
		{ id: 'nord', label: 'Nord', bg: '#2e3440', sidebar: '#3b4252', accent: '#81a1c1' },
		{ id: 'tokyo-night', label: 'Tokyo Night', bg: '#1a1b26', sidebar: '#16161e', accent: '#7aa2f7' },
		{ id: 'github-light', label: 'GitHub Light', bg: '#ffffff', sidebar: '#f6f8fa', accent: '#0969da' },
		{ id: 'github-dark', label: 'GitHub Dark', bg: '#0d1117', sidebar: '#161b22', accent: '#58a6ff' },
		{ id: 'dracula', label: 'Dracula', bg: '#282a36', sidebar: '#21222c', accent: '#bd93f9' },
		{ id: 'midnight-tide', label: 'Midnight Tide', bg: '#020122', sidebar: '#050a38', accent: '#2dd4bf' },
		{ id: 'blueberry', label: 'Blueberry', bg: '#1a1a2e', sidebar: '#16213e', accent: '#7b9cff' },
		{ id: 'forest-green', label: 'Forest Green', bg: '#1a2318', sidebar: '#1f2b1c', accent: '#6dbf67' },
		{ id: 'gruvbox', label: 'Gruvbox', bg: '#282828', sidebar: '#1d2021', accent: '#fabd2f' },
		{ id: 'cherry-blossom', label: 'Cherry Blossom', bg: '#1a0e12', sidebar: '#221218', accent: '#f4a0b5' },
		{ id: 'synthwave', label: 'Synthwave', bg: '#0d0221', sidebar: '#130332', accent: '#ff6ac1' },
		{ id: 'ember', label: 'Ember', bg: '#0f0a06', sidebar: '#1a1008', accent: '#ff8c42' },
		{ id: 'moonlit', label: 'Moonlit', bg: '#0a0d12', sidebar: '#0f1520', accent: '#b0cceb' },
		{ id: 'light-coffee', label: 'Light Coffee', bg: '#fdf8f3', sidebar: '#f5ede0', accent: '#8b5e3c' },
		{ id: 'dark-coffee', label: 'Dark Coffee', bg: '#1a1008', sidebar: '#211408', accent: '#c8894d' },
		{ id: 'cotton-candy', label: 'Cotton Candy', bg: '#fff0f8', sidebar: '#ffe8f4', accent: '#e85fa0' },
		{ id: 'crimson', label: 'Crimson', bg: '#1a0808', sidebar: '#220a0a', accent: '#e83535' },
		{ id: 'cloud', label: 'Cloud', bg: '#f0f8ff', sidebar: '#e4f2fc', accent: '#2879c8' },
		{ id: 'peach', label: 'Peach', bg: '#fff8f4', sidebar: '#ffeee4', accent: '#e8704a' },
		{ id: 'material-dark', label: 'Material Dark', bg: '#212121', sidebar: '#1b1b1b', accent: '#80cbc4' },
		{ id: 'material-light', label: 'Material Light', bg: '#fafafa', sidebar: '#f5f5f5', accent: '#00897b' },
		{ id: 'monokai', label: 'Monokai', bg: '#272822', sidebar: '#1e1f1c', accent: '#a6e22e' },
		{ id: 'rose-pine', label: 'Rosé Pine', bg: '#191724', sidebar: '#1f1d2e', accent: '#c4a7e7' },
		{ id: 'everforest', label: 'Everforest', bg: '#2d353b', sidebar: '#272e33', accent: '#83c092' },
		{ id: 'horizon', label: 'Horizon', bg: '#1c1e26', sidebar: '#16181f', accent: '#e9547f' },
		{ id: 'cyberpunk', label: 'Cyberpunk', bg: '#0a0e17', sidebar: '#060911', accent: '#ff0080' },
		{ id: 'black', label: 'Black', bg: '#000000', sidebar: '#0a0a0a', accent: '#ffffff' },
		{ id: 'one-dark', label: 'One Dark', bg: '#282c34', sidebar: '#21252b', accent: '#61afef' },
		{ id: 'matrix', label: 'Matrix', bg: '#0a0f0a', sidebar: '#060a06', accent: '#00ff66' },
		{ id: 'amber-crt', label: 'Amber CRT', bg: '#180f00', sidebar: '#120b00', accent: '#ffb000' },
		{ id: 'sepia', label: 'Sepia', bg: '#f4ecd8', sidebar: '#ebe0c7', accent: '#9a6a3a' },
		{ id: 'deep-ocean', label: 'Deep Ocean', bg: '#081821', sidebar: '#05121a', accent: '#35c4d4' },
		{ id: 'sandstone', label: 'Sandstone', bg: '#faf3e8', sidebar: '#f1e6d3', accent: '#c76b3f' },
		{ id: 'lavender', label: 'Lavender', bg: '#f6f3fc', sidebar: '#ece6f7', accent: '#7c5cc4' },
		{ id: 'graphite', label: 'Graphite', bg: '#17181a', sidebar: '#121315', accent: '#7f9cb3' },
		{ id: 'nebula', label: 'Nebula', bg: '#14102a', sidebar: '#0f0c21', accent: '#b388ff' },
		{ id: 'wine', label: 'Wine', bg: '#1c0f14', sidebar: '#160b10', accent: '#d98ca0' },
		{ id: 'jade', label: 'Jade', bg: '#0b1a15', sidebar: '#071410', accent: '#2bbd8e' },
	];

	// Keep the default themes pinned at the top and sort the rest alphabetically.
	const priorityThemeIds = ['system', 'light', 'dark'];
	themePresets.sort((a, b) => {
		const pa = priorityThemeIds.indexOf(a.id);
		const pb = priorityThemeIds.indexOf(b.id);
		if (pa !== -1 && pb !== -1) return pa - pb;
		if (pa !== -1) return -1;
		if (pb !== -1) return 1;
		return a.label.localeCompare(b.label);
	});

	const accentPresets = [
		{ name: 'Indigo', light: '#5b6abf', dark: '#7b9bd4' },
		{ name: 'Rose', light: '#e11d48', dark: '#d4768a' },
		{ name: 'Emerald', light: '#059669', dark: '#5bab8a' },
		{ name: 'Amber', light: '#d97706', dark: '#c9a04e' },
		{ name: 'Purple', light: '#7c3aed', dark: '#9a82c4' },
		{ name: 'Cyan', light: '#0891b2', dark: '#5ba8b5' },
		{ name: 'Orange', light: '#ea580c', dark: '#c98560' },
		{ name: 'Teal', light: '#0d9488', dark: '#5fada5' },
		{ name: 'Sakura', light: '#d4547a', dark: '#f4a0b5' },
		{ name: 'Gold', light: '#b8860b', dark: '#fabd2f' },
		{ name: 'Lavender', light: '#9d5cdb', dark: '#c084fc' },
		{ name: 'Sky', light: '#0284c7', dark: '#38bdf8' },
		{ name: 'Coral', light: '#e85d4a', dark: '#ff8a7a' },
	];

	const fontSizePresets = [
		{ label: 'Small', size: 13 },
		{ label: 'Default', size: 14 },
		{ label: 'Medium', size: 15 },
		{ label: 'Large', size: 16 },
		{ label: 'Extra Large', size: 18 },
	];

	const fontFamilyPresets = [
		{ name: 'System', value: 'system', stack: 'system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif' },
		{ name: 'Inter', value: 'inter', stack: '"Inter", -apple-system, BlinkMacSystemFont, sans-serif' },
		{ name: 'Georgia', value: 'georgia', stack: 'Georgia, "Times New Roman", serif' },
		{ name: 'Merriweather', value: 'merriweather', stack: '"Merriweather", Georgia, serif' },
		{ name: 'Lora', value: 'lora', stack: '"Lora", Georgia, serif' },
		{ name: 'Open Sans', value: 'opensans', stack: '"Open Sans", -apple-system, sans-serif' },
		{ name: 'Literata', value: 'literata', stack: '"Literata", Georgia, serif' },
		{ name: 'Mono', value: 'mono', stack: '"JetBrains Mono", "Fira Code", "Cascadia Code", monospace' },
		{ name: 'Ubuntu', value: 'ubuntu', stack: '"Ubuntu", -apple-system, sans-serif' },
		{ name: 'Ubuntu Mono', value: 'ubuntu-mono', stack: '"Ubuntu Mono", monospace' },
	];

	const lineHeightPresets = [
		{ label: 'Single', value: 1.0 },
		{ label: 'Tight', value: 1.4 },
		{ label: 'Default', value: 1.6 },
		{ label: 'Relaxed', value: 1.8 },
		{ label: 'Loose', value: 2.0 },
	];

	const contentWidthPresets: { label: string; value: number | null }[] = [
		{ label: 'Narrow', value: 600 },
		{ label: 'Medium', value: 760 },
		{ label: 'Wide', value: 960 },
		{ label: 'Full', value: null },
	];

	let activeAccent = $state($appConfig?.accent_color ?? 'Indigo');
	let customAccentColor = $state(($appConfig?.accent_color?.startsWith('#') ? $appConfig.accent_color : null) ?? '#5b6abf');
	let activeFontSize = $state($appConfig?.font_size ?? 14);
	let scrollToChangeFontSize = $state($appConfig?.scroll_to_change_font_size ?? true);
	let activeFontFamily = $state($appConfig?.font_family ?? 'system');
	let activeLineHeight = $state($appConfig?.line_height ?? 1.6);
	let activeUiScale = $state($appConfig?.ui_scale ?? defaultUiScale);
	let activeContentWidth = $state<number | null>($appConfig?.content_width ?? null);
	let themeDropdownOpen = $state(false);
	let accentDropdownOpen = $state(false);
	let activeCustomTheme = $derived($customThemes.find(c => c.id === $theme) ?? null);
	let activeThemePreset = $derived(
		themePresets.find(p => p.id === $theme) ?? themePresets.find(p => p.id === 'light')!
	);

	// Which theme each OS appearance maps to while `theme` is "system". Custom themes are offered
	// alongside the built-ins so a light/dark pair can be built out of them.
	let systemLightTheme = $state($appConfig?.system_light_theme ?? 'light');
	let systemDarkTheme = $state($appConfig?.system_dark_theme ?? 'dark');
	let systemPairOpen = $state<'light' | 'dark' | null>(null);

	let pairPresets = $derived([
		...themePresets.filter(p => p.id !== 'system'),
		...$customThemes.map(ct => ({
			id: ct.id,
			label: ct.name,
			bg: ct.colors.bg_primary,
			sidebar: ct.colors.bg_secondary,
			accent: ct.colors.text_primary,
		})),
	]);

	let systemPairRows = $derived([
		{ key: 'light' as const, title: 'Light appearance', selected: systemLightTheme },
		{ key: 'dark' as const, title: 'Dark appearance', selected: systemDarkTheme },
	]);

	function selectSystemPairTheme(which: 'light' | 'dark', id: string) {
		if (which === 'light') systemLightTheme = id;
		else systemDarkTheme = id;
		systemPairOpen = null;
		if ($appConfig) {
			$appConfig.system_light_theme = systemLightTheme;
			$appConfig.system_dark_theme = systemDarkTheme;
			$appConfig = { ...$appConfig };
		}
		setSystemThemes(systemLightTheme, systemDarkTheme).catch((e) =>
			console.error('Failed to save system themes:', e)
		);
	}
	let activeAccentPreset = $derived(accentPresets.find(p => p.name === activeAccent) ?? accentPresets[0]);
	let isCustomAccent = $derived(activeAccent.startsWith('#'));
	let activeAccentColor = $derived(
		isCustomAccent ? activeAccent : (isThemeDark ? activeAccentPreset.dark : activeAccentPreset.light)
	);

	// Custom theme editor state
	const DEFAULT_DARK_COLORS: CustomThemeColors = {
		bg_primary: '#1a1b26', bg_secondary: '#1f2029', bg_tertiary: '#2a2b3d',
		bg_hover: '#2f3145', bg_active: '#373a52', bg_editor: '#1a1b26',
		text_primary: '#c8cde4', text_secondary: '#6b7094', border_color: '#2f3145',
	};
	const DEFAULT_LIGHT_COLORS: CustomThemeColors = {
		bg_primary: '#ffffff', bg_secondary: '#f8f9fa', bg_tertiary: '#f0f2f5',
		bg_hover: '#e8eaed', bg_active: '#dde1e7', bg_editor: '#ffffff',
		text_primary: '#1a1a2e', text_secondary: '#6b7280', border_color: '#e2e5ea',
	};

	let customThemeEditing = $state<CustomTheme | null>(null);
	let customThemeEditorOpen = $state(false);
	let customThemeMessage = $state<{ type: 'success' | 'error'; text: string } | null>(null);
	let customThemeExportingId = $state<string | null>(null);
	let customThemeImporting = $state(false);

	function openNewCustomThemeEditor() {
		const isDark = isThemeDark;
		customThemeEditing = {
			id: `custom-${Date.now()}`,
			name: '',
			is_dark: isDark,
			colors: isDark ? { ...DEFAULT_DARK_COLORS } : { ...DEFAULT_LIGHT_COLORS },
		};
		customThemeEditorOpen = true;
	}

	function openEditCustomThemeEditor(ct: CustomTheme) {
		customThemeEditing = { ...ct, colors: { ...ct.colors } };
		customThemeEditorOpen = true;
	}

	function closeCustomThemeEditor() {
		customThemeEditorOpen = false;
		customThemeEditing = null;
	}

	function previewCustomTheme() {
		if (!customThemeEditing) return;
		const root = document.documentElement;
		root.style.setProperty('--bg-primary', customThemeEditing.colors.bg_primary);
		root.style.setProperty('--bg-secondary', customThemeEditing.colors.bg_secondary);
		root.style.setProperty('--bg-tertiary', customThemeEditing.colors.bg_tertiary);
		root.style.setProperty('--bg-hover', customThemeEditing.colors.bg_hover);
		root.style.setProperty('--bg-active', customThemeEditing.colors.bg_active);
		root.style.setProperty('--bg-editor', customThemeEditing.colors.bg_editor);
		root.style.setProperty('--text-primary', customThemeEditing.colors.text_primary);
		root.style.setProperty('--text-secondary', customThemeEditing.colors.text_secondary);
		root.style.setProperty('--border-color', customThemeEditing.colors.border_color);
		root.style.setProperty('--border-light', customThemeEditing.colors.border_color);
		root.style.setProperty('--text-tertiary', customThemeEditing.colors.text_secondary);
		root.classList.toggle('dark', customThemeEditing.is_dark);
	}

	function restoreCurrentTheme() {
		const root = document.documentElement;
		const varsToClear = ['--bg-primary','--bg-secondary','--bg-tertiary','--bg-hover','--bg-active','--bg-editor','--text-primary','--text-secondary','--border-color','--border-light','--text-tertiary'];
		root.classList.remove('dark');
		root.removeAttribute('data-theme');
		for (const v of varsToClear) root.style.removeProperty(v);
		const namedThemes = ['solarized-light','solarized-dark','catppuccin','nord','tokyo-night','github-light','github-dark','dracula','blueberry','forest-green','gruvbox','midnight-tide','cherry-blossom','synthwave','ember','moonlit','light-coffee','dark-coffee','cotton-candy','crimson','cloud','peach','material-dark','material-light','monokai','rose-pine','everforest','horizon','cyberpunk','black','one-dark','matrix','amber-crt','sepia','deep-ocean','sandstone','lavender','graphite','nebula','wine','jade'];
		if ($resolvedTheme.startsWith('custom-')) {
			const ct = $customThemes.find(c => c.id === $resolvedTheme);
			if (ct) {
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
				if (ct.is_dark) root.classList.add('dark');
			}
		} else if (namedThemes.includes($resolvedTheme)) {
			root.setAttribute('data-theme', $resolvedTheme);
			if (darkThemes.includes($resolvedTheme)) root.classList.add('dark');
		} else if ($resolvedTheme === 'dark') {
			root.classList.add('dark');
		}
	}

	async function saveCustomThemeEditor() {
		if (!customThemeEditing || !customThemeEditing.name.trim()) return;
		const theme_to_save = { ...customThemeEditing, name: customThemeEditing.name.trim() };
		try {
			await saveCustomTheme(theme_to_save);
			if ($appConfig) {
				const idx = $appConfig.custom_themes.findIndex(t => t.id === theme_to_save.id);
				if (idx >= 0) $appConfig.custom_themes[idx] = theme_to_save;
				else $appConfig.custom_themes = [...$appConfig.custom_themes, theme_to_save];
				$appConfig = { ...$appConfig };
			}
			closeCustomThemeEditor();
		} catch (e) {
			customThemeMessage = { type: 'error', text: String(e) };
		}
	}

	function cancelCustomThemeEditor() {
		restoreCurrentTheme();
		closeCustomThemeEditor();
	}

	function cancelCustomThemeFromOverlay(event: MouseEvent) {
		if (event.target === event.currentTarget) cancelCustomThemeEditor();
	}

	async function selectCustomTheme(ct: CustomTheme) {
		$theme = ct.id;
		setTheme(ct.id);
	}

	async function removeCustomTheme(ct: CustomTheme) {
		try {
			await deleteCustomTheme(ct.id);
			if (systemLightTheme === ct.id) systemLightTheme = 'light';
			if (systemDarkTheme === ct.id) systemDarkTheme = 'dark';
			if ($appConfig) {
				$appConfig = {
					...$appConfig,
					custom_themes: $appConfig.custom_themes.filter(t => t.id !== ct.id),
					system_light_theme: systemLightTheme,
					system_dark_theme: systemDarkTheme,
				};
			}
			if ($theme === ct.id) {
				$theme = 'system';
			}
		} catch (e) {
			customThemeMessage = { type: 'error', text: String(e) };
		}
	}

	async function handleExportCustomTheme(ct: CustomTheme) {
		customThemeExportingId = ct.id;
		try {
			const safeName = ct.name.replace(/[^a-z0-9]/gi, '-').toLowerCase();
			const path = await saveDialog({ filters: [{ name: 'JSON', extensions: ['json'] }], defaultPath: `${safeName}.json` });
			if (path) {
				await exportCustomTheme(ct.id, path);
				customThemeMessage = { type: 'success', text: `"${ct.name}" exported.` };
			}
		} catch (e) {
			customThemeMessage = { type: 'error', text: String(e) };
		} finally {
			customThemeExportingId = null;
		}
	}

	async function handleImportCustomThemes() {
		customThemeImporting = true;
		try {
			const path = await openDialog({ filters: [{ name: 'JSON', extensions: ['json'] }], multiple: false });
			if (path) {
				const imported = await importCustomThemes(path as string);
				if ($appConfig) {
					for (const t of imported) {
						const idx = $appConfig.custom_themes.findIndex(x => x.id === t.id);
						if (idx >= 0) $appConfig.custom_themes[idx] = t;
						else $appConfig.custom_themes.push(t);
					}
					$appConfig = { ...$appConfig };
				}
				customThemeMessage = { type: 'success', text: `Imported ${imported.length} theme${imported.length !== 1 ? 's' : ''}.` };
			}
		} catch (e) {
			customThemeMessage = { type: 'error', text: String(e) };
		} finally {
			customThemeImporting = false;
		}
	}

	// General settings
	let compactNotes = $state($appConfig?.compact_notes ?? false);
	let showNoteDates = $state($appConfig?.show_note_dates ?? true);
	let showNoteSwitcher = $state($appConfig?.show_note_switcher ?? false);
	let startupView = $state<StartupView>(normalizeStartupView($appConfig?.startup_view));
	let restoreLastSession = $state($appConfig?.restore_last_session ?? false);
	let timeFormat = $state($appConfig?.time_format ?? 'relative');
	let weekStart = $state($appConfig?.week_start ?? 'monday');
	let dailyTitleFormat = $state($appConfig?.daily_title_format ?? 'localized');
	let gpuAcceleration = $state($appConfig?.gpu_acceleration ?? true);
	let autostart = $state($appConfig?.autostart ?? false);

	// Editor settings
	let pdfPreview = $state($appConfig?.pdf_preview ?? false);
	let pdfHeight = $state($appConfig?.pdf_height ?? 600);
	let titleMode = $state($appConfig?.title_mode ?? 'input');
	let hideTitleInBody = $state($appConfig?.hide_title_in_body ?? false);
	let showLineNumbers = $state($appConfig?.show_line_numbers ?? false);
	let showLinkArrows = $state($appConfig?.show_link_arrows ?? true);
	let defaultViewMode = $state($appConfig?.default_view_mode ?? false);
	let newNotesInSourceMode = $state($appConfig?.new_notes_in_source_mode ?? false);
	let showTrayIcon = $state($appConfig?.show_tray_icon ?? false);
	let closeToTray = $state($appConfig?.close_to_tray ?? false);
	let enableWikiLinks = $state($appConfig?.enable_wiki_links ?? true);

	// Sidebar item visibility
	let showAllNotes = $state($appConfig?.show_all_notes ?? true);
	let showQuickAccess = $state($appConfig?.show_quick_access ?? true);
	let showTasks = $state($appConfig?.show_tasks ?? true);
	let showDailyNotes = $state($appConfig?.show_daily_notes ?? true);
	let showTrash = $state($appConfig?.show_trash ?? true);

	const pdfHeightPresets = [
		{ label: 'Small', value: 300 },
		{ label: 'Medium', value: 450 },
		{ label: 'Default', value: 600 },
		{ label: 'Large', value: 800 },
		{ label: 'Full', value: 1000 },
	];

	// Import state
	let importLoading = $state(false);
	let importResult = $state<ImportResult | null>(null);
	let importError = $state<string | null>(null);
	async function runObsidianImport() {
		importLoading = true;
		importResult = null;
		importError = null;

		const unlistenDone = await listen<{ success: boolean; files_converted?: number; links_converted?: number; frontmatter_normalized?: number; syntax_converted?: number; attachments_moved?: number; error?: string }>('import-done', (event) => {
			const data = event.payload;
			if (data.success) {
				importResult = {
					files_converted: data.files_converted ?? 0,
					links_converted: data.links_converted ?? 0,
					frontmatter_normalized: data.frontmatter_normalized ?? 0,
					syntax_converted: data.syntax_converted ?? 0,
					attachments_moved: data.attachments_moved ?? 0,
				};
			} else {
				importError = data.error ?? 'Import failed';
			}
			importLoading = false;
			unlistenDone();
		});

		try {
			await importObsidian();
		} catch (e) {
			importError = String(e);
			importLoading = false;
			unlistenDone();
		}
	}

	function saveGeneralSettings() {
		if ($appConfig) {
			$appConfig.compact_notes = compactNotes;
			$appConfig.show_note_dates = showNoteDates;
			$appConfig.show_note_switcher = showNoteSwitcher;
			$appConfig.startup_view = startupView;
			$appConfig.restore_last_session = restoreLastSession;
			$appConfig.time_format = timeFormat;
			$appConfig.week_start = weekStart;
			$appConfig.daily_title_format = dailyTitleFormat;
			$appConfig.gpu_acceleration = gpuAcceleration;
			$appConfig.autostart = autostart;
			$appConfig.pdf_preview = pdfPreview;
			$appConfig.pdf_height = pdfHeight;
			$appConfig.title_mode = titleMode;
			$appConfig.hide_title_in_body = hideTitleInBody;
			$appConfig.show_line_numbers = showLineNumbers;
			$appConfig.show_link_arrows = showLinkArrows;
			$appConfig.default_view_mode = defaultViewMode;
			$appConfig.new_notes_in_source_mode = newNotesInSourceMode;
			$appConfig.show_tray_icon = showTrayIcon;
			$appConfig.close_to_tray = closeToTray;
			$appConfig.enable_wiki_links = enableWikiLinks;
			$appConfig.show_all_notes = showAllNotes;
			$appConfig.show_quick_access = showQuickAccess;
			$appConfig.show_tasks = showTasks;
			$appConfig.show_daily_notes = showDailyNotes;
			$appConfig.show_trash = showTrash;
			}
			setGeneralSettings(compactNotes, timeFormat, weekStart, dailyTitleFormat, gpuAcceleration, autostart, pdfPreview, pdfHeight, titleMode, hideTitleInBody, showLineNumbers, showLinkArrows, defaultViewMode, newNotesInSourceMode, showTrayIcon, closeToTray, enableWikiLinks, showNoteDates, showNoteSwitcher, startupView, restoreLastSession, showAllNotes, showQuickAccess, showTasks, showDailyNotes, showTrash)
			.catch((e) => console.error('Failed to save general settings:', e));
	}

	function applyLinkArrows() {
		document.documentElement.classList.toggle('no-link-arrows', !showLinkArrows);
	}

	function selectTheme(preset: typeof themePresets[0]) {
		$theme = preset.id;
		setTheme(preset.id);
		themeDropdownOpen = false;
		// "System" has no palette of its own, it defers to whichever paired theme is active, so it
		// must not overwrite the accent the way a concrete theme does.
		if (preset.id !== 'system') selectCustomAccent(preset.accent);
	}

	function selectAccent(preset: typeof accentPresets[0]) {
		activeAccent = preset.name;
		applyAccent(preset);
		saveConfig(preset.name);
	}

	function applyAccent(preset: typeof accentPresets[0]) {
		const root = document.documentElement;
		const isDark = isThemeDark;
		const color = isDark ? preset.dark : preset.light;

		root.style.setProperty('--accent', color);
		root.style.setProperty('--text-accent', color);

		root.style.setProperty('--accent-hover', color);
		if (isDark) {
			root.style.setProperty('--accent-light', `color-mix(in srgb, ${color} 10%, transparent)`);
		} else {
			root.style.setProperty('--accent-light', `color-mix(in srgb, ${color} 8%, transparent)`);
		}
	}

	function saveConfig(accentName: string) {
		if ($appConfig) {
			$appConfig.accent_color = accentName;
		}
		setAccentColor(accentName).catch((e) => console.error('Failed to save accent:', e));
	}

	function applyCustomAccent(hex: string) {
		const root = document.documentElement;
		const isDark = isThemeDark;
		root.style.setProperty('--accent', hex);
		root.style.setProperty('--text-accent', hex);
		root.style.setProperty('--accent-hover', hex);
		root.style.setProperty('--accent-light', `color-mix(in srgb, ${hex} ${isDark ? '10%' : '8%'}, transparent)`);
	}

	function selectCustomAccent(hex: string) {
		customAccentColor = hex;
		activeAccent = hex;
		applyCustomAccent(hex);
		if ($appConfig) $appConfig.accent_color = hex;
		setAccentColor(hex).catch((e) => console.error('Failed to save custom accent:', e));
	}

	function selectFontSize(size: number) {
		activeFontSize = size;
		applyFontSize(size);
		if ($appConfig) {
			$appConfig.font_size = size;
			$appConfig.preferred_font_size = size;
		}
		setFontSize(size, true).catch((e) => console.error('Failed to save font size:', e));
	}

	function toggleScrollToChangeFontSize() {
		scrollToChangeFontSize = !scrollToChangeFontSize;
		if ($appConfig) $appConfig.scroll_to_change_font_size = scrollToChangeFontSize;
		setScrollToChangeFontSize(scrollToChangeFontSize)
			.catch((e) => console.error('Failed to save scroll font size setting:', e));
	}

	function applyFontSize(size: number) {
		document.documentElement.style.setProperty('--editor-font-size', `${size}px`);
	}

	function selectFontFamily(preset: typeof fontFamilyPresets[0]) {
		activeFontFamily = preset.value;
		applyFontFamily(preset.stack);
		if ($appConfig) $appConfig.font_family = preset.value;
		setFontFamily(preset.value).catch((e) => console.error('Failed to save font family:', e));
	}

	function applyFontFamily(stack: string) {
		document.documentElement.style.setProperty('--editor-font-family', stack);
	}

	function selectLineHeight(value: number) {
		activeLineHeight = value;
		applyLineHeight(value);
		if ($appConfig) $appConfig.line_height = value;
		setLineHeight(value).catch((e) => console.error('Failed to save line height:', e));
	}

	function applyLineHeight(value: number) {
		document.documentElement.style.setProperty('--editor-line-height', String(value));
	}

	function selectUiScale(value: number) {
		activeUiScale = value;
		applyUiScale(value);
		if ($appConfig) $appConfig.ui_scale = value;
		setUiScale(value).catch((e) => console.error('Failed to save interface scale:', e));
	}

	function selectContentWidth(value: number | null) {
		activeContentWidth = value;
		applyContentWidth(value);
		if ($appConfig) $appConfig.content_width = value;
		setContentWidth(value).catch((e) => console.error('Failed to save content width:', e));
	}

	function applyContentWidth(value: number | null) {
		document.documentElement.style.setProperty('--editor-content-width', value ? `${value}px` : 'none');
	}

	function clickOutside(node: HTMLElement, callback: () => void) {
		function handle(e: MouseEvent) {
			if (!node.contains(e.target as Node)) callback();
		}
		document.addEventListener('mousedown', handle, true);
		return { destroy() { document.removeEventListener('mousedown', handle, true); } };
	}

	function close() {
		// Ensure AI settings are saved when closing (in case input wasn't blurred)
		// Only save if the key differs from what's already stored
		if (aiProvider && aiApiKey && aiApiKey !== ($appConfig?.ai_api_key ?? '')) {
			saveAiSettings();
		}
		$showSettings = false;
	}

	function closeSettingsFromOverlay(event: MouseEvent) {
		if (event.target === event.currentTarget) close();
	}

	function handleEscape(event: KeyboardEvent) {
		if (event.key !== 'Escape') return;
		event.preventDefault();
		event.stopPropagation();
		if (restoreConfirm) restoreConfirm = null;
		else if (customThemeEditorOpen) cancelCustomThemeEditor();
		else close();
	}

	function dismissRestoreConfirm(event: MouseEvent) {
		if (event.target === event.currentTarget) restoreConfirm = null;
	}

	// Apply saved settings on mount
	$effect(() => {
		const savedAccent = $appConfig?.accent_color;
		if (savedAccent) {
			if (savedAccent.startsWith('#')) {
				activeAccent = savedAccent;
				customAccentColor = savedAccent;
				applyCustomAccent(savedAccent);
			} else {
				const preset = accentPresets.find(p => p.name === savedAccent);
				if (preset) {
					activeAccent = savedAccent;
					applyAccent(preset);
				}
			}
		}
		scrollToChangeFontSize = $appConfig?.scroll_to_change_font_size ?? true;
		const savedSize = $appConfig?.font_size;
		if (savedSize) {
			activeFontSize = savedSize;
			applyFontSize(savedSize);
		}
		const savedFamily = $appConfig?.font_family;
		if (savedFamily) {
			const preset = fontFamilyPresets.find(p => p.value === savedFamily);
			if (preset) {
				activeFontFamily = savedFamily;
				applyFontFamily(preset.stack);
			}
		}
		const savedLineHeight = $appConfig?.line_height;
		if (savedLineHeight) {
			activeLineHeight = savedLineHeight;
			applyLineHeight(savedLineHeight);
		}
		const savedContentWidth = $appConfig?.content_width;
		if (savedContentWidth) {
			activeContentWidth = savedContentWidth;
			applyContentWidth(savedContentWidth);
		}
		// Sync general settings
		if ($appConfig) {
			compactNotes = $appConfig.compact_notes ?? false;
			showNoteDates = $appConfig.show_note_dates ?? true;
			showNoteSwitcher = $appConfig.show_note_switcher ?? false;
			startupView = normalizeStartupView($appConfig.startup_view);
			restoreLastSession = $appConfig.restore_last_session ?? false;
			timeFormat = $appConfig.time_format ?? 'relative';
			weekStart = $appConfig.week_start ?? 'monday';
			dailyTitleFormat = $appConfig.daily_title_format ?? 'localized';
			gpuAcceleration = $appConfig.gpu_acceleration ?? true;
			autostart = $appConfig.autostart ?? false;
			pdfPreview = $appConfig.pdf_preview ?? false;
			pdfHeight = $appConfig.pdf_height ?? 600;
			titleMode = $appConfig.title_mode ?? 'input';
			newNotesInSourceMode = $appConfig.new_notes_in_source_mode ?? false;
			showAllNotes = $appConfig.show_all_notes ?? true;
			showQuickAccess = $appConfig.show_quick_access ?? true;
			showTasks = $appConfig.show_tasks ?? true;
			showDailyNotes = $appConfig.show_daily_notes ?? true;
			showTrash = $appConfig.show_trash ?? true;
		}
	});

	// Re-apply when the concrete theme changes, including an OS appearance switch.
	$effect(() => {
		const _ = $resolvedTheme;
		const preset = accentPresets.find(p => p.name === activeAccent);
		if (preset) {
			applyAccent(preset);
		}
	});
</script>

{#if $showSettings}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="settings-overlay" class:mobile={isMobile} onclick={closeSettingsFromOverlay} onkeydowncapture={handleEscape} onkeydown={handleEscape}>
		<div class="settings-panel" class:mobile={isMobile} role="dialog" aria-modal="true" aria-labelledby="settings-title" tabindex="-1">
			<div class="settings-header">
				<h2 id="settings-title">Settings</h2>
				<button class="close-btn" onclick={close} aria-label="Close settings">
					<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						<line x1="18" y1="6" x2="6" y2="18" />
						<line x1="6" y1="6" x2="18" y2="18" />
					</svg>
				</button>
			</div>

			<div class="settings-content">
				<nav class="settings-tabs">
					<button class="tab-btn" class:active={activeTab === 'general'} onclick={() => activeTab = 'general'}>
						<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
							<path d="M12.22 2h-.44a2 2 0 00-2 2v.18a2 2 0 01-1 1.73l-.43.25a2 2 0 01-2 0l-.15-.08a2 2 0 00-2.73.73l-.22.38a2 2 0 00.73 2.73l.15.1a2 2 0 011 1.72v.51a2 2 0 01-1 1.74l-.15.09a2 2 0 00-.73 2.73l.22.38a2 2 0 002.73.73l.15-.08a2 2 0 012 0l.43.25a2 2 0 011 1.73V20a2 2 0 002 2h.44a2 2 0 002-2v-.18a2 2 0 011-1.73l.43-.25a2 2 0 012 0l.15.08a2 2 0 002.73-.73l.22-.39a2 2 0 00-.73-2.73l-.15-.08a2 2 0 01-1-1.74v-.5a2 2 0 011-1.74l.15-.09a2 2 0 00.73-2.73l-.22-.38a2 2 0 00-2.73-.73l-.15.08a2 2 0 01-2 0l-.43-.25a2 2 0 01-1-1.73V4a2 2 0 00-2-2z"/><circle cx="12" cy="12" r="3"/>
						</svg>
						General
					</button>
					<button class="tab-btn" class:active={activeTab === 'editor'} onclick={() => activeTab = 'editor'}>
						<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
							<path d="M21.174 6.812a1 1 0 00-3.986-3.987L3.842 16.174a2 2 0 00-.5.83l-1.321 4.352a.5.5 0 00.623.622l4.353-1.32a2 2 0 00.83-.497z"/><path d="M15 5l4 4"/>
						</svg>
						Editor
					</button>
					<button class="tab-btn" class:active={activeTab === 'styling'} onclick={() => activeTab = 'styling'}>
						<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
							<path d="M12 2v4"/><path d="m6.8 14-3.5 2"/><path d="m20.7 16-3.5-2"/><path d="M6.8 10 3.3 8"/><path d="m20.7 8-3.5 2"/><path d="m9 22 3-8 3 8"/><path d="M8 22h8"/><circle cx="12" cy="12" r="2"/>
						</svg>
						Styling
					</button>
					{#if !isMobile}
					<button class="tab-btn" class:active={activeTab === 'import'} onclick={() => activeTab = 'import'}>
						<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
							<path d="M12 3v12"/><path d="m8 11 4 4 4-4"/><path d="M8 5H4a2 2 0 00-2 2v10a2 2 0 002 2h16a2 2 0 002-2V7a2 2 0 00-2-2h-4"/>
						</svg>
						Import
					</button>
				{/if}
					{#if !isMobile}
					<button class="tab-btn" class:active={activeTab === 'backup'} onclick={() => { activeTab = 'backup'; loadBackups(); }}>
						<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
							<path d="M21 12a9 9 0 00-9-9 9.75 9.75 0 00-6.74 2.74L3 8"/><path d="M3 3v5h5"/><path d="M3 12a9 9 0 009 9 9.75 9.75 0 006.74-2.74L21 16"/><path d="M16 16h5v5"/>
						</svg>
						Backup
					</button>
					{/if}
					<button class="tab-btn" class:active={activeTab === 'maintenance'} onclick={() => activeTab = 'maintenance'}>
						<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
							<line x1="22" x2="2" y1="12" y2="12"/><path d="M5.45 5.11 2 12v6a2 2 0 002 2h16a2 2 0 002-2v-6l-3.45-6.89A2 2 0 0016.76 4H7.24a2 2 0 00-1.79 1.11z"/><line x1="6" x2="6.01" y1="16" y2="16"/><line x1="10" x2="10.01" y1="16" y2="16"/>
						</svg>
						Maintenance
					</button>
					<button class="tab-btn" class:active={activeTab === 'ai'} onclick={() => activeTab = 'ai'}>
						<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
							<path d="M12 8V4l-2-2"/><rect x="4" y="8" width="16" height="12" rx="2"/><path d="M2 14h2"/><path d="M20 14h2"/><path d="M9 13v2"/><path d="M15 13v2"/>
						</svg>
						AI
					</button>
					<button class="tab-btn" class:active={activeTab === 'sync'} onclick={() => activeTab = 'sync'}>
						<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
							<path d="M21 2v6h-6"/><path d="M3 12a9 9 0 0115-6.7L21 8"/><path d="M3 22v-6h6"/><path d="M21 12a9 9 0 01-15 6.7L3 16"/>
						</svg>
						Sync
					</button>
					{#if !isAndroid}
					<button class="tab-btn" class:active={activeTab === 'updates'} onclick={() => activeTab = 'updates'}>
						<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
							<path d="M21 12a9 9 0 00-9-9 9.75 9.75 0 00-6.74 2.74L3 8"/><path d="M3 3v5h5"/><path d="M12 7v5l3 3"/>
						</svg>
						Updates
					</button>
					{/if}
				</nav>

				<div class="settings-body">
					{#if activeTab === 'general'}
						<div class="tab-content">
							<div class="settings-section">
								<h3>Notes List</h3>
								<label class="setting-toggle">
									<span class="setting-label">
										<span class="setting-name">Compact mode</span>
										<span class="setting-desc">Show notes in a denser layout without preview</span>
									</span>
									<button class="toggle-switch" class:on={compactNotes} role="switch" aria-checked={compactNotes} aria-label="Compact notes" onclick={() => { compactNotes = !compactNotes; saveGeneralSettings(); }}>
										<span class="toggle-knob"></span>
									</button>
								</label>
								<label class="setting-toggle">
									<span class="setting-label">
										<span class="setting-name">Show dates</span>
										<span class="setting-desc">Show the date next to each note in the list</span>
									</span>
									<button class="toggle-switch" class:on={showNoteDates} role="switch" aria-checked={showNoteDates} aria-label="Show note dates" onclick={() => { showNoteDates = !showNoteDates; saveGeneralSettings(); }}>
										<span class="toggle-knob"></span>
									</button>
								</label>
							</div>

							<div class="settings-section">
								<h3>Notebooks</h3>
								<div class="setting-toggle">
									<span class="setting-label">
										<span class="setting-name">Sort order</span>
										<span class="setting-desc">Alphabetical (default) or manual. Drag notebooks above/below each other to reorder.</span>
									</span>
								</div>
								<div class="setting-options" style="margin-top: 8px;">
									<button class="option-btn" class:active={$notebookSortMode === 'alphabetical'} onclick={() => { $notebookSortMode = 'alphabetical'; }}>Alphabetical</button>
									<button class="option-btn" class:active={$notebookSortMode === 'manual'} onclick={() => { $notebookSortMode = 'manual'; }}>Manual</button>
								</div>
							</div>

							{#if !isMobile}
							<div class="settings-section">
								<h3>Sidebar</h3>
								<p class="setting-desc" style="margin: 0 0 10px;">Hide navigation items you don't use, leaving search and the notebook tree. Any item you hide stays reachable from the command palette ({modKey}+P).</p>
								<label class="setting-toggle">
									<span class="setting-label"><span class="setting-name">All Notes</span></span>
									<button class="toggle-switch" class:on={showAllNotes} role="switch" aria-checked={showAllNotes} aria-label="Show All Notes" onclick={() => { showAllNotes = !showAllNotes; saveGeneralSettings(); }}>
										<span class="toggle-knob"></span>
									</button>
								</label>
								<label class="setting-toggle">
									<span class="setting-label"><span class="setting-name">Quick Access</span></span>
									<button class="toggle-switch" class:on={showQuickAccess} role="switch" aria-checked={showQuickAccess} aria-label="Show Quick Access" onclick={() => { showQuickAccess = !showQuickAccess; saveGeneralSettings(); }}>
										<span class="toggle-knob"></span>
									</button>
								</label>
								<label class="setting-toggle">
									<span class="setting-label"><span class="setting-name">Tasks</span></span>
									<button class="toggle-switch" class:on={showTasks} role="switch" aria-checked={showTasks} aria-label="Show Tasks" onclick={() => { showTasks = !showTasks; saveGeneralSettings(); }}>
										<span class="toggle-knob"></span>
									</button>
								</label>
								<label class="setting-toggle">
									<span class="setting-label"><span class="setting-name">Daily Notes</span></span>
									<button class="toggle-switch" class:on={showDailyNotes} role="switch" aria-checked={showDailyNotes} aria-label="Show Daily Notes" onclick={() => { showDailyNotes = !showDailyNotes; saveGeneralSettings(); }}>
										<span class="toggle-knob"></span>
									</button>
								</label>
								<label class="setting-toggle">
									<span class="setting-label"><span class="setting-name">Trash</span></span>
									<button class="toggle-switch" class:on={showTrash} role="switch" aria-checked={showTrash} aria-label="Show Trash" onclick={() => { showTrash = !showTrash; saveGeneralSettings(); }}>
										<span class="toggle-knob"></span>
									</button>
								</label>
							</div>
							{/if}

							{#if !$platformIsMobile}
							<div class="settings-section">
								<h3>Interface</h3>
								<label class="setting-toggle">
									<span class="setting-label">
										<span class="setting-name">Show note switcher in title bar</span>
										<span class="setting-desc">Show recent and Quick Access notes in the title bar.</span>
									</span>
									<button class="toggle-switch" class:on={showNoteSwitcher} role="switch" aria-checked={showNoteSwitcher} aria-label="Show note switcher in title bar" onclick={() => { showNoteSwitcher = !showNoteSwitcher; saveGeneralSettings(); }}>
										<span class="toggle-knob"></span>
									</button>
								</label>
							</div>
							{/if}

							<div class="settings-section">
								<h3>Startup</h3>
								<div class="setting-label">
									<span class="setting-name">Default view</span>
									<span class="setting-desc">Used when session restore is off or the previous view is unavailable.</span>
								</div>
								<div class="setting-options" style="margin-top: 8px;">
									<button class="option-btn" class:active={startupView === 'all'} onclick={() => { startupView = 'all'; saveGeneralSettings(); }}>All Notes</button>
									<button class="option-btn" class:active={startupView === 'quickaccess'} onclick={() => { startupView = 'quickaccess'; saveGeneralSettings(); }}>Quick Access</button>
									<button class="option-btn" class:active={startupView === 'tasks'} onclick={() => { startupView = 'tasks'; saveGeneralSettings(); }}>Tasks</button>
									<button class="option-btn" class:active={startupView === 'daily'} onclick={() => { startupView = 'daily'; saveGeneralSettings(); }}>Daily Notes</button>
								</div>
								<label class="setting-toggle" style="margin-top: 12px;">
									<span class="setting-label">
										<span class="setting-name">Restore last session on launch</span>
										<span class="setting-desc">Reopen the last view and note when possible. This overrides the default view.</span>
									</span>
									<button class="toggle-switch" class:on={restoreLastSession} role="switch" aria-checked={restoreLastSession} aria-label="Restore last session on launch" onclick={() => { restoreLastSession = !restoreLastSession; saveGeneralSettings(); }}>
										<span class="toggle-knob"></span>
									</button>
								</label>
							</div>

							<div class="settings-section">
								<h3>Time Format</h3>
								<div class="setting-options">
									<button class="option-btn" class:active={timeFormat === 'relative'} onclick={() => { timeFormat = 'relative'; saveGeneralSettings(); }}>Relative</button>
									<button class="option-btn" class:active={timeFormat === '12h'} onclick={() => { timeFormat = '12h'; saveGeneralSettings(); }}>12-hour</button>
									<button class="option-btn" class:active={timeFormat === '24h'} onclick={() => { timeFormat = '24h'; saveGeneralSettings(); }}>24-hour</button>
								</div>
							</div>

							<div class="settings-section">
								<h3>Start week on</h3>
								<div class="setting-options">
									<button class="option-btn" class:active={weekStart === 'monday'} onclick={() => { weekStart = 'monday'; saveGeneralSettings(); }}>Monday</button>
									<button class="option-btn" class:active={weekStart === 'sunday'} onclick={() => { weekStart = 'sunday'; saveGeneralSettings(); }}>Sunday</button>
								</div>
							</div>

							<div class="settings-section">
								<h3>Daily note title format</h3>
								<div class="setting-options">
									<button class="option-btn" class:active={dailyTitleFormat === 'localized'} onclick={() => { dailyTitleFormat = 'localized'; saveGeneralSettings(); }}>Localized</button>
									<button class="option-btn" class:active={dailyTitleFormat === 'iso'} onclick={() => { dailyTitleFormat = 'iso'; saveGeneralSettings(); }}>2026-06-10</button>
									<button class="option-btn" class:active={dailyTitleFormat === 'long'} onclick={() => { dailyTitleFormat = 'long'; saveGeneralSettings(); }}>June 10, 2026</button>
									<button class="option-btn" class:active={dailyTitleFormat === 'us'} onclick={() => { dailyTitleFormat = 'us'; saveGeneralSettings(); }}>06/10/2026</button>
									<button class="option-btn" class:active={dailyTitleFormat === 'eu'} onclick={() => { dailyTitleFormat = 'eu'; saveGeneralSettings(); }}>10/06/2026</button>
								</div>
							</div>

							{#if isMobile}
							<div class="settings-section">
								<h3>Vault</h3>
								<p class="setting-desc" style="margin-bottom: 12px; color: var(--text-tertiary); font-size: 13px;">Current: <strong style="color: var(--text-primary);">{$appConfig?.active_vault?.split('/').pop() ?? 'Unknown'}</strong></p>
								<button class="import-btn" onclick={async () => { if (!(await onBeforeVaultSwitch())) return; $showSettings = false; $vaultReady = false; }}>
									<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
										<rect x="3" y="3" width="18" height="16" rx="2"/>
										<circle cx="12" cy="11" r="3.5"/>
										<path d="M12 7.5V6M12 16v-1.5M7 19v2M17 19v2"/>
									</svg>
									Manage Vaults
								</button>
							</div>
							{/if}


							{#if !isMobile}
							<div class="settings-section">
								<h3>Performance</h3>
								<label class="setting-toggle">
									<span class="setting-label">
										<span class="setting-name">GPU Acceleration</span>
										<span class="setting-desc">Use hardware acceleration for rendering (requires restart)</span>
									</span>
									<button class="toggle-switch" class:on={gpuAcceleration} role="switch" aria-checked={gpuAcceleration} aria-label="GPU acceleration" onclick={() => { gpuAcceleration = !gpuAcceleration; saveGeneralSettings(); }}>
										<span class="toggle-knob"></span>
									</button>
								</label>
							</div>

							<div class="settings-section">
								<h3>System</h3>
								<label class="setting-toggle">
									<span class="setting-label">
										<span class="setting-name">Start at system startup</span>
										<span class="setting-desc">Launch HelixNotes when your computer starts</span>
									</span>
									<button class="toggle-switch" class:on={autostart} role="switch" aria-checked={autostart} aria-label="Start at system startup" onclick={() => { autostart = !autostart; saveGeneralSettings(); }}>
										<span class="toggle-knob"></span>
									</button>
								</label>
								<label class="setting-toggle">
									<span class="setting-label">
										<span class="setting-name">Show in system tray</span>
										<span class="setting-desc">Show an icon in the notification area (requires restart)</span>
									</span>
									<button class="toggle-switch" class:on={showTrayIcon} role="switch" aria-checked={showTrayIcon} aria-label="Show in system tray" onclick={() => { showTrayIcon = !showTrayIcon; if (!showTrayIcon) closeToTray = false; saveGeneralSettings(); }}>
										<span class="toggle-knob"></span>
									</button>
								</label>
								{#if showTrayIcon}
								<label class="setting-toggle">
									<span class="setting-label">
										<span class="setting-name">Close to tray</span>
										<span class="setting-desc">Minimize to tray instead of quitting when closing the window</span>
									</span>
									<button class="toggle-switch" class:on={closeToTray} role="switch" aria-checked={closeToTray} aria-label="Close to tray" onclick={() => { closeToTray = !closeToTray; saveGeneralSettings(); }}>
										<span class="toggle-knob"></span>
									</button>
								</label>
								{/if}
							</div>
						{/if}
						</div>
					{:else if activeTab === 'maintenance'}
						<div class="tab-content">
							<div class="settings-section">
								<h3>Vault Overview</h3>
								{#if vaultStats}
									<div class="maintenance-stats">
										<div class="maintenance-stat-row">
											<span class="maintenance-stat-label">Notes</span>
											<span class="maintenance-stat-value">{vaultStats.total_notes}</span>
										</div>
										<div class="maintenance-stat-row">
											<span class="maintenance-stat-label">Attachments</span>
											<span class="maintenance-stat-value">{vaultStats.total_attachments}</span>
										</div>
										<div class="maintenance-stat-row">
											<span class="maintenance-stat-label">Notes size</span>
											<span class="maintenance-stat-value">{formatVaultSize(vaultStats.notes_size)}</span>
										</div>
										<div class="maintenance-stat-row">
											<span class="maintenance-stat-label">Attachments size</span>
											<span class="maintenance-stat-value">{formatVaultSize(vaultStats.attachments_size)}</span>
										</div>
										<div class="maintenance-stat-row maintenance-stat-total">
											<span class="maintenance-stat-label">Total vault size</span>
											<span class="maintenance-stat-value">{formatVaultSize(vaultStats.total_size)}</span>
										</div>
									</div>
								{:else if vaultStatsError}
									<p class="setting-desc">Vault statistics could not be loaded.</p>
								{:else}
									<p class="setting-desc">Loading vault statistics…</p>
								{/if}
							</div>

							<div class="settings-section">
								<h3>Orphaned Attachments</h3>
								<div class="vault-maintenance">
									<p class="setting-desc">Review attachment files not referenced by saved notes or icon settings. Save changes in other note windows before scanning.</p>
									{#if orphanError}<p class="cleanup-message" role="alert">{orphanError}</p>{/if}
									{#if orphanedAttachments === null}
										<button class="import-btn cleanup-action" onclick={scanOrphanedAttachments} disabled={scanningOrphanedAttachments}>
											{#if scanningOrphanedAttachments}
												<svg class="spinner-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10" opacity="0.25" /><path d="M12 2a10 10 0 019.95 9" /></svg>
												Scanning…
											{:else}
												Find orphaned attachments
											{/if}
										</button>
									{:else if orphanedAttachments.length === 0}
										<p class="cleanup-message">No orphaned attachments. Nothing to clean up.</p>
									{:else}
										<p class="cleanup-message">{orphanedAttachments.length} {orphanedAttachments.length === 1 ? 'attachment is' : 'attachments are'} not referenced by any note ({formatVaultSize(orphanedAttachmentTotal)}).</p>
										<div class="cleanup-list">
											{#each orphanedAttachments as attachment}
												<div class="cleanup-row">
													<span class="cleanup-name" title={attachment.name}>{attachment.name}</span>
													<span class="cleanup-size">{formatVaultSize(attachment.size)}</span>
												</div>
											{/each}
										</div>
										<button class="import-btn cleanup-action" onclick={trashFoundOrphanedAttachments} disabled={trashingOrphanedAttachments}>
											{#if trashingOrphanedAttachments}
												<svg class="spinner-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10" opacity="0.25" /><path d="M12 2a10 10 0 019.95 9" /></svg>
												Moving…
											{:else}
												Move {orphanedAttachments.length} to Trash
											{/if}
										</button>
										<p class="cleanup-hint">Moved to the vault's trash folder (recoverable), not permanently deleted.</p>
									{/if}
								</div>
							</div>
						</div>
					{:else if activeTab === 'editor'}
						<div class="tab-content">
							<div class="settings-section">
								<h3>Title</h3>
								<label class="setting-toggle">
									<span class="setting-label">
										<span class="setting-name">Hide title in note body</span>
										<span class="setting-desc">Hide the first heading when it matches the note title</span>
									</span>
									<button class="toggle-switch" class:on={hideTitleInBody} role="switch" aria-checked={hideTitleInBody} aria-label="Hide title in note body" onclick={() => { hideTitleInBody = !hideTitleInBody; saveGeneralSettings(); }}>
										<span class="toggle-knob"></span>
									</button>
								</label>
								<label class="setting-toggle" style="margin-top: 12px;">
									<span class="setting-label">
										<span class="setting-name">Show link arrows</span>
										<span class="setting-desc">Display the arrow icon after external links</span>
									</span>
									<button class="toggle-switch" class:on={showLinkArrows} role="switch" aria-checked={showLinkArrows} aria-label="Show link arrows" onclick={() => { showLinkArrows = !showLinkArrows; saveGeneralSettings(); applyLinkArrows(); }}>
										<span class="toggle-knob"></span>
									</button>
								</label>
								{#if !isMobile}
								<label class="setting-toggle" style="margin-top: 12px;">
									<span class="setting-label">
										<span class="setting-name">Show line numbers</span>
										<span class="setting-desc">Display line numbers in the markdown source editor</span>
									</span>
									<button class="toggle-switch" class:on={showLineNumbers} role="switch" aria-checked={showLineNumbers} aria-label="Show line numbers" onclick={() => { showLineNumbers = !showLineNumbers; saveGeneralSettings(); }}>
										<span class="toggle-knob"></span>
									</button>
								</label>
								{/if}
								<label class="setting-toggle" style="margin-top: 12px;">
									<span class="setting-label">
										<span class="setting-name">Open notes in View Mode</span>
										<span class="setting-desc">Notes open as read-only by default. Click the eye icon to switch to editing.</span>
									</span>
									<button class="toggle-switch" class:on={defaultViewMode} role="switch" aria-checked={defaultViewMode} aria-label="Open notes in View Mode" onclick={() => { defaultViewMode = !defaultViewMode; saveGeneralSettings(); }}>
										<span class="toggle-knob"></span>
									</button>
								</label>
								<label class="setting-toggle" style="margin-top: 12px;">
									<span class="setting-label">
										<span class="setting-name">Open new notes in source mode</span>
										<span class="setting-desc">Start empty notes in the plain-text Markdown editor instead of rich-text mode</span>
									</span>
									<button class="toggle-switch" class:on={newNotesInSourceMode} role="switch" aria-checked={newNotesInSourceMode} aria-label="Open new notes in source mode" onclick={() => { newNotesInSourceMode = !newNotesInSourceMode; saveGeneralSettings(); }}>
										<span class="toggle-knob"></span>
									</button>
								</label>
							</div>


							<div class="settings-section">
								<h3>Wiki Links & Graph</h3>
								<label class="setting-toggle">
									<span class="setting-label">
										<span class="setting-name">Enable wiki links</span>
										<span class="setting-desc">Link notes with [[Note Title]] syntax and visualize connections in a graph view</span>
									</span>
									<button class="toggle-switch" class:on={enableWikiLinks} role="switch" aria-checked={enableWikiLinks} aria-label="Enable wiki links" onclick={() => { enableWikiLinks = !enableWikiLinks; saveGeneralSettings(); }}>
										<span class="toggle-knob"></span>
									</button>
								</label>
							</div>


							{#if !isMobile}
							<div class="settings-section">
								<h3>PDF Preview</h3>
								<label class="setting-toggle">
									<span class="setting-label">
										<span class="setting-name">Inline PDF preview</span>
										<span class="setting-desc">Render embedded PDF files as inline previews inside notes</span>
									</span>
									<button class="toggle-switch" class:on={pdfPreview} role="switch" aria-checked={pdfPreview} aria-label="Inline PDF preview" onclick={() => { pdfPreview = !pdfPreview; saveGeneralSettings(); }}>
										<span class="toggle-knob"></span>
									</button>
								</label>
							</div>

							{#if pdfPreview}
								<div class="settings-section">
									<h3>PDF Height</h3>
									<div class="font-size-options">
										{#each pdfHeightPresets as preset}
											<button
												class="font-size-btn"
												class:active={pdfHeight === preset.value}
												onclick={() => { pdfHeight = preset.value; saveGeneralSettings(); }}
											>
												<span class="font-size-label">{preset.label}</span>
												<span class="font-size-value">{preset.value}px</span>
											</button>
										{/each}
									</div>
									<span class="setting-hint">Default height for PDF previews in notes</span>
								</div>
							{/if}
						{/if}
						</div>
					{:else if activeTab === 'styling'}
						<div class="tab-content">
							<div class="two-col-sections">
							<div class="settings-section">
								<h3>Theme</h3>
								<div class="dropdown-wrap">
									<button
										class="dropdown-trigger"
										class:open={themeDropdownOpen}
										onclick={() => { themeDropdownOpen = !themeDropdownOpen; accentDropdownOpen = false; }}
									>
										<span class="dropdown-preview" style="--preview-bg: {activeThemePreset.bg}; --preview-sidebar: {activeThemePreset.sidebar}; --preview-accent: {activeThemePreset.accent}">
											<span class="preview-sidebar"></span>
											<span class="preview-main">
												<span class="preview-accent-bar"></span>
											</span>
										</span>
										<span class="dropdown-trigger-label">{activeThemePreset.label}</span>
										<svg class="dropdown-chevron" viewBox="0 0 16 16" fill="none"><path d="M4 6l4 4 4-4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
									</button>
									{#if themeDropdownOpen}
										<div class="dropdown-list" use:clickOutside={() => themeDropdownOpen = false}>
											{#each themePresets as preset}
												<button
													class="dropdown-item"
													class:active={$theme === preset.id}
													onclick={() => selectTheme(preset)}
												>
													<span class="dropdown-preview" style="--preview-bg: {preset.bg}; --preview-sidebar: {preset.sidebar}; --preview-accent: {preset.accent}">
														<span class="preview-sidebar"></span>
														<span class="preview-main">
															<span class="preview-accent-bar"></span>
														</span>
													</span>
													<span class="dropdown-item-label">{preset.label}</span>
													{#if $theme === preset.id}
														<svg class="dropdown-check" viewBox="0 0 16 16" fill="none"><path d="M3 8l4 4 6-6" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
													{/if}
												</button>
											{/each}
										</div>
									{/if}
								</div>
								{#if $theme === 'system'}
									<span class="setting-hint">Follows the operating system appearance.</span>
								{/if}
							</div>

							<div class="settings-section">
								<h3>Accent Color</h3>
								<div class="dropdown-wrap">
									<button
										class="dropdown-trigger"
										class:open={accentDropdownOpen}
										onclick={() => { accentDropdownOpen = !accentDropdownOpen; themeDropdownOpen = false; }}
									>
										<span class="accent-dot" style="background: {activeAccentColor}"></span>
										<span class="dropdown-trigger-label">{isCustomAccent ? 'Custom' : activeAccentPreset.name}</span>
										<svg class="dropdown-chevron" viewBox="0 0 16 16" fill="none"><path d="M4 6l4 4 4-4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
									</button>
									{#if accentDropdownOpen}
										<div class="dropdown-list" use:clickOutside={() => accentDropdownOpen = false}>
											{#each accentPresets as preset}
												<button
													class="dropdown-item"
													class:active={activeAccent === preset.name}
													onclick={() => { selectAccent(preset); accentDropdownOpen = false; }}
												>
													<span class="accent-dot" style="background: {isThemeDark ? preset.dark : preset.light}"></span>
													<span class="dropdown-item-label">{preset.name}</span>
													{#if activeAccent === preset.name}
														<svg class="dropdown-check" viewBox="0 0 16 16" fill="none"><path d="M3 8l4 4 6-6" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
													{/if}
												</button>
											{/each}
											<div class="dropdown-divider"></div>
											<label class="dropdown-item custom-accent-item" class:active={isCustomAccent}>
												<span class="accent-dot" style="background: {customAccentColor}"></span>
												<span class="dropdown-item-label">Custom</span>
												<input
													type="color"
													class="color-wheel-input"
													value={customAccentColor}
													oninput={(e) => applyCustomAccent(e.currentTarget.value)}
													onchange={(e) => selectCustomAccent(e.currentTarget.value)}
												/>
												{#if isCustomAccent}
													<svg class="dropdown-check" viewBox="0 0 16 16" fill="none"><path d="M3 8l4 4 6-6" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
												{/if}
											</label>
										</div>
									{/if}
								</div>
							</div>

							{#if $theme === 'system'}
								{#each systemPairRows as row}
									<div class="settings-section">
										<h3>{row.title}</h3>
										<div class="dropdown-wrap">
											<button
												class="dropdown-trigger"
												class:open={systemPairOpen === row.key}
												onclick={() => { systemPairOpen = systemPairOpen === row.key ? null : row.key; themeDropdownOpen = false; accentDropdownOpen = false; }}
											>
												{#each pairPresets.filter(p => p.id === row.selected) as preset}
													<span class="dropdown-preview" style="--preview-bg: {preset.bg}; --preview-sidebar: {preset.sidebar}; --preview-accent: {preset.accent}">
														<span class="preview-sidebar"></span>
														<span class="preview-main">
															<span class="preview-accent-bar"></span>
														</span>
													</span>
													<span class="dropdown-trigger-label">{preset.label}</span>
												{/each}
												<svg class="dropdown-chevron" viewBox="0 0 16 16" fill="none"><path d="M4 6l4 4 4-4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
											</button>
											{#if systemPairOpen === row.key}
												<div class="dropdown-list" use:clickOutside={() => systemPairOpen = null}>
													{#each pairPresets as preset}
														<button
															class="dropdown-item"
															class:active={row.selected === preset.id}
															onclick={() => selectSystemPairTheme(row.key, preset.id)}
														>
															<span class="dropdown-preview" style="--preview-bg: {preset.bg}; --preview-sidebar: {preset.sidebar}; --preview-accent: {preset.accent}">
																<span class="preview-sidebar"></span>
																<span class="preview-main">
																	<span class="preview-accent-bar"></span>
																</span>
															</span>
															<span class="dropdown-item-label">{preset.label}</span>
															{#if row.selected === preset.id}
																<svg class="dropdown-check" viewBox="0 0 16 16" fill="none"><path d="M3 8l4 4 6-6" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
															{/if}
														</button>
													{/each}
												</div>
											{/if}
										</div>
									</div>
								{/each}
							{/if}
							</div>

							<!-- Custom Themes -->
							<div class="settings-section">
								<div class="section-header-row">
									<h3>Custom Themes</h3>
									<button class="create-theme-btn" onclick={openNewCustomThemeEditor}>
										<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
										New Theme
									</button>
								</div>

								{#if $customThemes.length > 0}
									<div class="custom-theme-list">
										{#each $customThemes as ct}
											<div class="custom-theme-row" class:active={$theme === ct.id}>
												<button class="custom-theme-swatch-btn" onclick={() => selectCustomTheme(ct)} title="Apply {ct.name}">
													<span class="custom-theme-swatch" style="background: {ct.colors.bg_primary}; border-color: {ct.colors.border_color}">
														<span class="swatch-sidebar" style="background: {ct.colors.bg_secondary}"></span>
													</span>
													<span class="custom-theme-name">{ct.name}</span>
													{#if $theme === ct.id}
														<svg class="dropdown-check" viewBox="0 0 16 16" fill="none"><path d="M3 8l4 4 6-6" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
													{/if}
												</button>
												<div class="custom-theme-actions">
													<button class="icon-btn" title="Edit" onclick={() => openEditCustomThemeEditor(ct)}>
														<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21.174 6.812a1 1 0 00-3.986-3.987L3.842 16.174a2 2 0 00-.5.83l-1.321 4.352a.5.5 0 00.623.622l4.353-1.32a2 2 0 00.83-.497z"/></svg>
													</button>
													{#if !isMobile}
													<button class="icon-btn" title="Export" disabled={customThemeExportingId === ct.id} onclick={() => handleExportCustomTheme(ct)}>
														<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>
													</button>
													{/if}
													<button class="icon-btn danger" title="Delete" onclick={() => removeCustomTheme(ct)}>
														<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/></svg>
													</button>
												</div>
											</div>
										{/each}
									</div>
								{:else}
									<p class="setting-hint" style="margin-top: 8px;">No custom themes yet. Create one to define your own colors.</p>
								{/if}

								{#if !isMobile}
									<div class="custom-theme-io">
										<button class="backup-link-btn" onclick={handleImportCustomThemes} disabled={customThemeImporting}>
											<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" y1="3" x2="12" y2="15"/></svg>
											{customThemeImporting ? 'Importing...' : 'Import theme'}
										</button>
									</div>
								{/if}

								{#if customThemeMessage}
									<div class="import-result {customThemeMessage.type}" style="margin-top: 10px;">
										{#if customThemeMessage.type === 'success'}
											<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 11.08V12a10 10 0 11-5.93-9.14"/><polyline points="22 4 12 14.01 9 11.01"/></svg>
										{:else}
											<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><line x1="15" y1="9" x2="9" y2="15"/><line x1="9" y1="9" x2="15" y2="15"/></svg>
										{/if}
										<span>{customThemeMessage.text}</span>
									</div>
								{/if}
							</div>

							<!-- Custom Theme Editor Modal -->
							{#if customThemeEditorOpen && customThemeEditing}
								<!-- svelte-ignore a11y_no_static_element_interactions -->
								<div class="custom-theme-modal-overlay" onclick={cancelCustomThemeFromOverlay} onkeydown={handleEscape}>
									<div class="custom-theme-modal" role="dialog" aria-modal="true" aria-labelledby="custom-theme-title" tabindex="-1">
										<div class="custom-theme-modal-header">
											<h3 id="custom-theme-title">{customThemeEditing.id.startsWith('custom-') && $customThemes.some(c => c.id === customThemeEditing!.id) ? 'Edit Theme' : 'New Custom Theme'}</h3>
											<button class="close-btn" onclick={cancelCustomThemeEditor} aria-label="Close theme editor">
												<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
											</button>
										</div>

										<div class="custom-theme-modal-body">
											<div class="ct-field">
												<label class="ct-label" for="custom-theme-name">Theme Name</label>
												<input id="custom-theme-name" class="ct-name-input" type="text" placeholder="My Theme" bind:value={customThemeEditing.name} maxlength={40} />
											</div>

											<div class="ct-field">
												<span class="ct-label">Mode</span>
												<div class="setting-options">
													<button class="option-btn" class:active={!customThemeEditing.is_dark} onclick={() => { if (customThemeEditing) { customThemeEditing.is_dark = false; previewCustomTheme(); } }}>Light</button>
													<button class="option-btn" class:active={customThemeEditing.is_dark} onclick={() => { if (customThemeEditing) { customThemeEditing.is_dark = true; previewCustomTheme(); } }}>Dark</button>
												</div>
											</div>

											<div class="ct-colors-grid">
												<div class="ct-color-group">
													<span class="ct-group-label">Backgrounds</span>
													{#each [
														{ key: 'bg_primary', label: 'Primary' },
														{ key: 'bg_secondary', label: 'Sidebar' },
														{ key: 'bg_tertiary', label: 'Cards' },
														{ key: 'bg_editor', label: 'Editor' },
														{ key: 'bg_hover', label: 'Hover' },
														{ key: 'bg_active', label: 'Active' },
													] as field}
														<label class="ct-color-row">
															<input
																type="color"
																class="ct-color-input"
																value={customThemeEditing.colors[field.key as keyof CustomThemeColors]}
																oninput={(e) => { if (customThemeEditing) { (customThemeEditing.colors as any)[field.key] = e.currentTarget.value; previewCustomTheme(); } }}
															/>
															<span class="ct-color-label">{field.label}</span>
														</label>
													{/each}
												</div>

												<div class="ct-color-group">
													<span class="ct-group-label">Text &amp; Borders</span>
													{#each [
														{ key: 'text_primary', label: 'Primary Text' },
														{ key: 'text_secondary', label: 'Secondary Text' },
														{ key: 'border_color', label: 'Borders' },
													] as field}
														<label class="ct-color-row">
															<input
																type="color"
																class="ct-color-input"
																value={customThemeEditing.colors[field.key as keyof CustomThemeColors]}
																oninput={(e) => { if (customThemeEditing) { (customThemeEditing.colors as any)[field.key] = e.currentTarget.value; previewCustomTheme(); } }}
															/>
															<span class="ct-color-label">{field.label}</span>
														</label>
													{/each}
												</div>
											</div>

											<div class="ct-preview">
												<span class="ct-preview-label">Preview</span>
												<div class="ct-preview-window" style="--p-bg: {customThemeEditing.colors.bg_primary}; --p-sidebar: {customThemeEditing.colors.bg_secondary}; --p-card: {customThemeEditing.colors.bg_tertiary}; --p-text: {customThemeEditing.colors.text_primary}; --p-text2: {customThemeEditing.colors.text_secondary}; --p-border: {customThemeEditing.colors.border_color}; --p-hover: {customThemeEditing.colors.bg_hover};">
													<div class="ct-preview-sidebar">
														<div class="ct-preview-item active"></div>
														<div class="ct-preview-item"></div>
														<div class="ct-preview-item"></div>
													</div>
													<div class="ct-preview-main">
														<div class="ct-preview-title"></div>
														<div class="ct-preview-line long"></div>
														<div class="ct-preview-line medium"></div>
														<div class="ct-preview-line short"></div>
													</div>
												</div>
											</div>
										</div>

										<div class="custom-theme-modal-footer">
											<button class="cancel-btn" onclick={cancelCustomThemeEditor}>Cancel</button>
											<button class="save-btn" onclick={saveCustomThemeEditor} disabled={!customThemeEditing.name.trim()}>Save Theme</button>
										</div>
									</div>
								</div>
							{/if}

							<div class="settings-section">
								<h3>Font Size</h3>
								<div class="font-size-options">
									{#each fontSizePresets as preset}
										<button
											class="font-size-btn"
											class:active={activeFontSize === preset.size}
											onclick={() => selectFontSize(preset.size)}
										>
											<span class="font-size-label">{preset.label}</span>
											<span class="font-size-value">{preset.size}px</span>
										</button>
									{/each}
								</div>
								{#if !isMobile}
									<label class="setting-toggle">
										<span class="setting-label">
											<span class="setting-name">Scroll to change font size</span>
											<span class="setting-desc">Allow Cmd/Ctrl + scroll to resize editor text</span>
										</span>
										<button class="toggle-switch" class:on={scrollToChangeFontSize} role="switch" aria-checked={scrollToChangeFontSize} aria-label="Scroll to change font size" onclick={toggleScrollToChangeFontSize}>
											<span class="toggle-knob"></span>
										</button>
									</label>
								{/if}
							</div>

							{#if !isMobile || isAndroid}
								<div class="settings-section">
									<h3>Interface Scale</h3>
									<div class="font-size-options">
										{#each uiScalePresets as preset}
											<button
												class="font-size-btn"
												class:active={activeUiScale === preset.value}
												onclick={() => selectUiScale(preset.value)}
											>
												<span class="font-size-label">{preset.label}</span>
											</button>
										{/each}
									</div>
									<p class="setting-hint">Zooms the whole app: every panel, menu, and the editor. Use Default ({Math.round(defaultUiScale * 100)}%) to reset.</p>
								</div>
							{/if}

							{#if !isMobile}
								<div class="settings-section">
									<h3>Note Width</h3>
									<div class="font-size-options">
										{#each contentWidthPresets as preset}
											<button
												class="font-size-btn"
												class:active={activeContentWidth === preset.value}
												onclick={() => selectContentWidth(preset.value)}
											>
												<span class="font-size-label">{preset.label}</span>
											</button>
										{/each}
									</div>
									<p class="setting-hint">Caps the width of the note text for easier reading on wide screens; the column stays centered. Most noticeable in Focus Mode. "Full" uses the entire width.</p>
								</div>
							{/if}

							<div class="settings-section">
								<h3>Line Spacing</h3>
								<div class="font-size-options">
									{#each lineHeightPresets as preset}
										<button
											class="font-size-btn"
											class:active={activeLineHeight === preset.value}
											onclick={() => selectLineHeight(preset.value)}
										>
											<span class="font-size-label">{preset.label}</span>
											<span class="font-size-value">{preset.value}</span>
										</button>
									{/each}
								</div>
								<p class="setting-hint">Sets the spacing in the editor. Text copied to Word or email is pasted with single spacing.</p>
							</div>

							<div class="settings-section">
								<h3>Font</h3>
								<div class="font-family-options">
									{#each fontFamilyPresets as preset}
										<button
											class="font-family-btn"
											class:active={activeFontFamily === preset.value}
											onclick={() => selectFontFamily(preset)}
										>
											<span class="font-family-preview" style="font-family: {preset.stack}">Aa</span>
											<span class="font-family-name">{preset.name}</span>
										</button>
									{/each}
								</div>
							</div>
						</div>
					{:else if activeTab === 'backup'}
						<div class="tab-content">
							<div class="settings-section">
								<h3>Automatic Backup</h3>

								<label class="setting-toggle">
									<span>Enable automatic backup</span>
									<button class="toggle-switch" class:on={$appConfig?.backup_enabled} role="switch" aria-checked={$appConfig?.backup_enabled ?? false} aria-label="Enable automatic backup" onclick={() => { if ($appConfig) { $appConfig = { ...$appConfig, backup_enabled: !$appConfig.backup_enabled }; saveBackupSettings(); } }}>
										<span class="toggle-knob"></span>
									</button>
								</label>

								<div class="backup-setting-group">
									<span class="backup-setting-label">Backup frequency</span>
									<div class="setting-options">
										{#each [{ value: '1h', label: '1 hour' }, { value: '6h', label: '6 hours' }, { value: '12h', label: '12 hours' }, { value: '24h', label: '24 hours' }] as opt}
											<button class="option-btn" class:active={($appConfig?.backup_frequency ?? '24h') === opt.value} onclick={() => { if ($appConfig) { $appConfig = { ...$appConfig, backup_frequency: opt.value }; saveBackupSettings(); } }}>{opt.label}</button>
										{/each}
									</div>
								</div>

								<div class="backup-setting-group">
									<span class="backup-setting-label">Maximum backups</span>
									<div class="setting-options">
										{#each [5, 10, 20, 50] as count}
											<button class="option-btn" class:active={($appConfig?.backup_max_count ?? 10) === count} onclick={() => { if ($appConfig) { $appConfig = { ...$appConfig, backup_max_count: count }; saveBackupSettings(); } }}>{count}</button>
										{/each}
									</div>
								</div>

								<label class="setting-toggle">
									<span class="setting-label">
										<span class="setting-name">Include attachments</span>
										<span class="setting-desc">Include images and files in backups (increases size significantly)</span>
									</span>
									<button class="toggle-switch" class:on={$appConfig?.backup_include_attachments} role="switch" aria-checked={$appConfig?.backup_include_attachments ?? false} aria-label="Include attachments in backups" onclick={() => { if ($appConfig) { $appConfig = { ...$appConfig, backup_include_attachments: !$appConfig.backup_include_attachments }; saveBackupSettings(); } }}>
										<span class="toggle-knob"></span>
									</button>
								</label>

								<div class="backup-actions">
									{#if !isMobile}
									<button class="backup-link-btn" onclick={handleSelectBackupFolder}>
										<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
											<path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/>
										</svg>
										{$appConfig?.backup_location ? 'Change backup folder' : 'Select backup folder'}
									</button>
									{#if $appConfig?.backup_location}
										<p class="backup-path">{$appConfig.backup_location}</p>
									{/if}
									{/if}
									<button class="backup-link-btn" onclick={handleBackupNow} disabled={backupLoading}>
										{#if backupLoading}
											<svg class="spinner-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10" opacity="0.25" /><path d="M12 2a10 10 0 019.95 9" /></svg>
											Backing up...
										{:else}
											<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
												<path d="M19 21H5a2 2 0 01-2-2V5a2 2 0 012-2h11l5 5v11a2 2 0 01-2 2z" />
												<polyline points="17 21 17 13 7 13 7 21" />
												<polyline points="7 3 7 8 15 8" />
											</svg>
											Backup now
										{/if}
									</button>
								</div>

								<p class="backup-last-time">
									{#if lastBackupTime}
										Last backup: {formatBackupDate(lastBackupTime)}
									{:else}
										No backups yet
									{/if}
								</p>

								{#if backupMessage}
									<div class="import-result {backupMessage.type}">
										{#if backupMessage.type === 'success'}
											<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 11.08V12a10 10 0 11-5.93-9.14" /><polyline points="22 4 12 14.01 9 11.01" /></svg>
										{:else}
											<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10" /><line x1="15" y1="9" x2="9" y2="15" /><line x1="9" y1="9" x2="15" y2="15" /></svg>
										{/if}
										<span>{backupMessage.text}</span>
									</div>
								{/if}
							</div>

							<div class="settings-section">
								<h3>Restore Backup</h3>
								{#if backups.length === 0}
									<p class="backup-empty">No backups found</p>
								{:else}
									<div class="backup-list">
										{#each backups as entry}
											<div class="backup-item">
												<div class="backup-info">
													<span class="backup-date">{formatBackupDate(entry.created)}</span>
													<span class="backup-size">{formatBackupSize(entry.size)}</span>
													{#if entry.other_vault}
														<span class="backup-size" title="This backup was made from a different vault folder">other vault</span>
													{/if}
												</div>
												<div class="backup-item-actions">
													<button class="backup-action-btn" title="Restore" onclick={() => restoreConfirm = entry}>
														<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
															<polyline points="1 4 1 10 7 10" />
															<path d="M3.51 15a9 9 0 102.13-9.36L1 10" />
														</svg>
													</button>
													<button class="backup-action-btn danger" title="Delete" onclick={() => handleDeleteBackup(entry)}>
														<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
															<polyline points="3 6 5 6 21 6" />
															<path d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2" />
														</svg>
													</button>
												</div>
											</div>
										{/each}
									</div>
								{/if}
							</div>

							{#if restoreConfirm}
								<!-- svelte-ignore a11y_no_static_element_interactions -->
								<div class="restore-confirm-overlay" onclick={dismissRestoreConfirm} onkeydown={handleEscape}>
									<div class="restore-confirm" role="alertdialog" aria-modal="true" aria-labelledby="restore-confirm-title" tabindex="-1">
										<h4 id="restore-confirm-title">Restore Backup?</h4>
										<p>This will replace all notes in your vault with the backup from <strong>{formatBackupDate(restoreConfirm.created)}</strong>. This action cannot be undone.</p>
										{#if restoreConfirm.other_vault}
											<p><strong>This backup was made from a different vault folder.</strong> Restoring it replaces this vault's notes with that vault's. Continue only if this vault was moved or renamed since then.</p>
										{/if}
										<div class="restore-confirm-actions">
											<button class="restore-cancel" onclick={() => restoreConfirm = null}>Cancel</button>
											<button class="restore-confirm-btn" onclick={() => handleRestore(restoreConfirm!)}>Restore</button>
										</div>
									</div>
								</div>
							{/if}
						</div>

					{:else if activeTab === 'import'}
						<div class="tab-content">
							<div class="settings-section">
								<h3>Import from Obsidian</h3>
								<p class="import-desc">Fully converts an Obsidian vault to HelixNotes format: normalizes frontmatter (tags, IDs, dates, title headings), converts <code>[[wiki-links]]</code> to standard markdown links, converts <code>==highlights==</code> and <code>%%comments%%</code>, and relocates attachments to <code>.helixnotes/attachments/</code>. Open the Obsidian vault directory as your HelixNotes vault first.</p>
								<div class="import-warn">
									<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z" /><line x1="12" y1="9" x2="12" y2="13" /><line x1="12" y1="17" x2="12.01" y2="17" /></svg>
									<span>This modifies files in place. Make a backup before running!</span>
								</div>
								<button class="import-btn" onclick={runObsidianImport} disabled={importLoading}>
									{#if importLoading}
										<svg class="spinner-icon" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10" opacity="0.25" /><path d="M12 2a10 10 0 019.95 9" /></svg>
										Importing...
									{:else}
										<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
											<path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4" />
											<polyline points="7 10 12 15 17 10" />
											<line x1="12" y1="15" x2="12" y2="3" />
										</svg>
										Import Obsidian Vault
									{/if}
								</button>

							{#if importResult}
								<div class="import-result success">
									<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 11.08V12a10 10 0 11-5.93-9.14" /><polyline points="22 4 12 14.01 9 11.01" /></svg>
									<div class="import-stats">
										<span>Import complete</span>
										<ul>
											{#if importResult.frontmatter_normalized}<li>Normalized frontmatter in {importResult.frontmatter_normalized} notes</li>{/if}
											{#if importResult.links_converted}<li>Converted {importResult.links_converted} wiki-links to markdown</li>{/if}
											{#if importResult.syntax_converted}<li>Converted Obsidian syntax (highlights, comments) in {importResult.syntax_converted} notes</li>{/if}
											{#if importResult.attachments_moved}<li>Moved {importResult.attachments_moved} attachments to .helixnotes/attachments/</li>{/if}
											{#if !importResult.frontmatter_normalized && !importResult.links_converted && !importResult.syntax_converted && !importResult.attachments_moved}<li>No changes needed</li>{/if}
										</ul>
									</div>
								</div>
							{/if}
								{#if importError}
									<div class="import-result error">
										<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10" /><line x1="15" y1="9" x2="9" y2="15" /><line x1="9" y1="9" x2="15" y2="15" /></svg>
										<span>{importError}</span>
									</div>
								{/if}
							</div>
						</div>

					{:else if activeTab === 'ai'}
						<div class="tab-content">
							<div class="settings-section">
								<h3>Provider</h3>
								<div class="setting-options" style="flex-wrap: wrap;">
									<button class="option-btn" class:active={!aiProvider} onclick={() => { if (!aiProvider) return; aiProvider = null; aiTestMessage = null; saveAiSettings(); }}>Disabled</button>
									<button class="option-btn" class:active={aiProvider === 'ollama'} onclick={() => { if (aiProvider === 'ollama') return; aiProvider = 'ollama'; aiModel = 'gemma3:4b'; aiTestMessage = null; saveAiSettings(); }}>Ollama</button>
									<button class="option-btn" class:active={aiProvider === 'anthropic'} onclick={() => { if (aiProvider === 'anthropic') return; aiProvider = 'anthropic'; aiModel = 'claude-sonnet-5'; aiTestMessage = null; saveAiSettings(); }}>Anthropic</button>
									<button class="option-btn" class:active={aiProvider === 'openai'} onclick={() => { if (aiProvider === 'openai') return; aiProvider = 'openai'; aiModel = 'gpt-6-sol'; aiTestMessage = null; saveAiSettings(); }}>OpenAI</button>
									<button class="option-btn" class:active={aiProvider === 'openai_compatible'} onclick={() => { if (aiProvider === 'openai_compatible') return; aiProvider = 'openai_compatible'; aiModel = ''; aiTestMessage = null; saveAiSettings(); }}>OpenAI Compatible</button>
								</div>
							</div>

							{#if aiProvider}
								{#if aiProvider === 'ollama' || aiProvider === 'openai_compatible'}
									<p class="setting-hint" style="color: var(--text-success, #4ade80); margin-top: -4px; margin-bottom: 12px;">Your data stays on your device. No text is sent to any external server.</p>
								{:else}
									<p class="setting-hint" style="color: var(--text-warning, #f59e0b); margin-top: -4px; margin-bottom: 12px;">Your selected text and note content will be sent to {aiProvider === 'openai' ? 'OpenAI' : 'Anthropic'} servers for processing.</p>
								{/if}

								{#if aiProvider === 'ollama'}
									<div class="settings-section">
										<h3>Server URL</h3>
										<input
											type="text"
											class="ai-key-input"
											placeholder="http://localhost:11434"
											value={_ollamaBaseUrl}
											oninput={(e) => { _ollamaBaseUrl = (e.target as HTMLInputElement).value; }}
											onblur={saveAiSettings}
										/>
										<p class="setting-hint">Ollama server address. Install from <a href="https://ollama.com" target="_blank" class="ai-link">ollama.com</a></p>
									</div>
									<div class="settings-section">
										<h3>API Key <span style="font-weight: 400; text-transform: none; font-size: 11px;">(optional)</span></h3>
										<div class="ai-key-row">
											<input
												type={aiShowOllamaKey ? 'text' : 'password'}
												class="ai-key-input"
												placeholder="Only required if your server requires auth"
												value={_ollamaApiKey}
												oninput={(e) => { _ollamaApiKey = (e.target as HTMLInputElement).value; }}
												onblur={saveAiSettings}
											/>
											<button class="ai-key-toggle" onclick={() => aiShowOllamaKey = !aiShowOllamaKey} title={aiShowOllamaKey ? 'Hide' : 'Show'}>
												{#if aiShowOllamaKey}
													<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17.94 17.94A10.07 10.07 0 0112 20c-7 0-11-8-11-8a18.45 18.45 0 015.06-5.94M9.9 4.24A9.12 9.12 0 0112 4c7 0 11 8 11 8a18.5 18.5 0 01-2.16 3.19m-6.72-1.07a3 3 0 11-4.24-4.24"/><line x1="1" y1="1" x2="23" y2="23"/></svg>
												{:else}
													<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/><circle cx="12" cy="12" r="3"/></svg>
												{/if}
											</button>
										</div>
									</div>
								{:else if aiProvider === 'openai_compatible'}
									<div class="settings-section">
										<h3>Server URL</h3>
										<input
											type="text"
											class="ai-key-input"
											placeholder="http://localhost:1234"
											value={_openaiCompatibleBaseUrl}
											oninput={(e) => { _openaiCompatibleBaseUrl = (e.target as HTMLInputElement).value; }}
											onblur={saveAiSettings}
										/>
										<p class="setting-hint">Base URL for any OpenAI-compatible server (LM Studio, LocalAI, vLLM, Nvidia NIM, etc.). Enter the server root; the app appends <code>/v1/chat/completions</code> (a trailing <code>/v1</code> is fine too).</p>
									</div>
									<div class="settings-section">
										<h3>API Key <span style="font-weight: 400; text-transform: none; font-size: 11px;">(optional)</span></h3>
										<div class="ai-key-row">
											<input
												type={aiShowCompatibleKey ? 'text' : 'password'}
												class="ai-key-input"
												placeholder="Leave empty if no auth required"
												value={_openaiCompatibleKey}
												oninput={(e) => { _openaiCompatibleKey = (e.target as HTMLInputElement).value; }}
												onblur={saveAiSettings}
											/>
											<button class="ai-key-toggle" onclick={() => aiShowCompatibleKey = !aiShowCompatibleKey} title={aiShowCompatibleKey ? 'Hide' : 'Show'}>
												{#if aiShowCompatibleKey}
													<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17.94 17.94A10.07 10.07 0 0112 20c-7 0-11-8-11-8a18.45 18.45 0 015.06-5.94M9.9 4.24A9.12 9.12 0 0112 4c7 0 11 8 11 8a18.5 18.5 0 01-2.16 3.19m-6.72-1.07a3 3 0 11-4.24-4.24"/><line x1="1" y1="1" x2="23" y2="23"/></svg>
												{:else}
													<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/><circle cx="12" cy="12" r="3"/></svg>
												{/if}
											</button>
										</div>
									</div>
								{:else}
									<div class="settings-section">
										<h3>API Key</h3>
										<div class="ai-key-row">
											<input
												type={aiShowKey ? 'text' : 'password'}
												class="ai-key-input"
												placeholder={aiProvider === 'openai' ? 'sk-...' : 'sk-ant-...'}
												value={aiApiKey}
												oninput={(e) => { const v = (e.target as HTMLInputElement).value; if (aiProvider === 'openai') _openaiKey = v; else _anthropicKey = v; }}
												onblur={saveAiSettings}
											/>
											<button class="ai-key-toggle" onclick={() => aiShowKey = !aiShowKey} title={aiShowKey ? 'Hide' : 'Show'}>
												{#if aiShowKey}
													<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17.94 17.94A10.07 10.07 0 0112 20c-7 0-11-8-11-8a18.45 18.45 0 015.06-5.94M9.9 4.24A9.12 9.12 0 0112 4c7 0 11 8 11 8a18.5 18.5 0 01-2.16 3.19m-6.72-1.07a3 3 0 11-4.24-4.24"/><line x1="1" y1="1" x2="23" y2="23"/></svg>
												{:else}
													<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/><circle cx="12" cy="12" r="3"/></svg>
												{/if}
											</button>
										</div>
										{#if aiProvider === 'openai'}
											<p class="setting-hint">Get your API key from <a href="https://platform.openai.com/api-keys" target="_blank" class="ai-link">platform.openai.com</a></p>
										{:else}
											<p class="setting-hint">Get your API key from <a href="https://console.anthropic.com/settings/keys" target="_blank" class="ai-link">console.anthropic.com</a></p>
										{/if}
									</div>
								{/if}

								<div class="settings-section">
									<h3>Model</h3>
									{#if aiProvider === 'ollama' || aiProvider === 'openai_compatible'}
										<input
											type="text"
											class="ai-key-input"
											placeholder={aiProvider === 'ollama' ? 'gemma3:4b' : 'Enter model name'}
											value={aiModel}
											oninput={(e) => { aiModel = (e.target as HTMLInputElement).value; }}
											onblur={saveAiSettings}
										/>
										<p class="setting-hint">{aiProvider === 'ollama' ? 'Enter the model name as shown by <code>ollama list</code>' : 'Enter the model name as required by your server'}</p>
									{:else}
										<div class="ai-model-options">
											{#each aiModels as m}
												<button
													class="ai-model-btn"
													class:active={aiModel === m.value}
													onclick={() => { aiModel = m.value; saveAiSettings(); }}
												>
													<span class="ai-model-name">{m.label}</span>
													<span class="ai-model-desc">{m.desc}</span>
												</button>
											{/each}
										</div>
										{#if aiModel && !aiModels.some(model => model.value === aiModel)}
											<p class="setting-hint">Saved model: <code>{aiModel}</code>. It remains selected until you choose another model.</p>
										{/if}
									{/if}
								</div>

								<div class="settings-section">
									<h3>Writing Style</h3>
									<textarea
										class="ai-style-input"
										placeholder="e.g. Professional and concise. Use active voice. Avoid jargon."
										value={aiWritingStyle}
										oninput={(e) => { aiWritingStyle = (e.target as HTMLTextAreaElement).value; }}
										onblur={saveAiSettings}
										rows="3"
									></textarea>
									<p class="setting-hint">Describe your preferred tone, style, or personality. This will be applied to all AI actions.</p>
								</div>

								<div class="settings-section">
									<h3>Connection</h3>
									<button class="import-btn" onclick={handleTestAi} disabled={aiTestLoading || (aiProvider === 'openai_compatible' && !_openaiCompatibleBaseUrl) || (aiProvider !== 'ollama' && aiProvider !== 'openai_compatible' && !aiApiKey)}>
										{#if aiTestLoading}
											<svg class="spinner-icon" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10" opacity="0.25" /><path d="M12 2a10 10 0 019.95 9" /></svg>
											Testing...
										{:else}
											<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 11.08V12a10 10 0 11-5.93-9.14" /><polyline points="22 4 12 14.01 9 11.01" /></svg>
											Test Connection
										{/if}
									</button>
									{#if aiTestMessage}
										<div class="import-result {aiTestMessage.type}">
											{#if aiTestMessage.type === 'success'}
												<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 11.08V12a10 10 0 11-5.93-9.14" /><polyline points="22 4 12 14.01 9 11.01" /></svg>
											{:else}
												<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10" /><line x1="15" y1="9" x2="9" y2="15" /><line x1="9" y1="9" x2="15" y2="15" /></svg>
											{/if}
											<span>{aiTestMessage.text}</span>
										</div>
									{/if}
								</div>

								<div class="settings-section">
									<h3>Usage</h3>
									{#if isMobile}
									<p class="import-desc">Use the AI button in the toolbar or header to access AI writing tools: improve, fix grammar, rewrite, summarize, translate, and more.</p>
								{:else}
									<p class="import-desc">Select text in the editor and right-click to access AI writing tools: improve, fix grammar, rewrite, summarize, translate, and more.</p>
								{/if}
								</div>
							{/if}
						</div>

					{:else if activeTab === 'sync'}
						<div class="tab-content">
							<div class="settings-section">
								<h3>Provider</h3>
								<div class="setting-options">
									<button class="option-btn" class:active={!syncProvider} onclick={() => { syncProvider = null; syncMessage = null; syncTestMessage = null; saveSyncSettings(); }}>Disabled</button>
									<button class="option-btn" class:active={syncProvider === 'webdav'} onclick={() => { syncProvider = 'webdav'; syncMessage = null; syncTestMessage = null; saveSyncSettings(); }}>WebDAV</button>
								</div>
								<p class="setting-hint">Sync your vault to your own WebDAV server (Nextcloud, ownCloud, a NAS).</p>
							</div>

							{#if syncProvider === 'webdav'}
								<div class="settings-section">
									<h3>Server URL</h3>
									<div class="ai-key-row">
										<input type="text" class="ai-key-input" placeholder="https://cloud.example.com/remote.php/dav/files/USER/HelixNotes" value={syncUrl} oninput={(e) => { syncUrl = (e.target as HTMLInputElement).value; }} onblur={saveSyncSettings} />
									</div>
									<p class="setting-hint" style="overflow-x: auto;">WebDAV folder URL, e.g. <code style="white-space: nowrap;">https://your-server/remote.php/dav/files/USER/Folder</code></p>
								</div>
								<div class="settings-section">
									<h3>Username</h3>
									<div class="ai-key-row">
										<input type="text" class="ai-key-input" placeholder="your username" value={syncUsername} oninput={(e) => { syncUsername = (e.target as HTMLInputElement).value; }} onblur={saveSyncSettings} />
									</div>
								</div>
								<div class="settings-section">
									<h3>Password</h3>
									<div class="ai-key-row">
										<input type={syncShowPassword ? 'text' : 'password'} class="ai-key-input" placeholder="app password" value={syncPassword} oninput={(e) => { syncPassword = (e.target as HTMLInputElement).value; }} onblur={saveSyncSettings} />
										<button class="ai-key-toggle" onclick={() => syncShowPassword = !syncShowPassword} title={syncShowPassword ? 'Hide' : 'Show'}>
											{#if syncShowPassword}
												<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17.94 17.94A10.07 10.07 0 0112 20c-7 0-11-8-11-8a18.45 18.45 0 015.06-5.94M9.9 4.24A9.12 9.12 0 0112 4c7 0 11 8 11 8a18.5 18.5 0 01-2.16 3.19m-6.72-1.07a3 3 0 11-4.24-4.24"/><line x1="1" y1="1" x2="23" y2="23"/></svg>
											{:else}
												<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/><circle cx="12" cy="12" r="3"/></svg>
											{/if}
										</button>
									</div>
									<p class="setting-hint">Use an app password, not your main one. Stored locally on this device.</p>
								</div>

								<div class="settings-section">
									<h3>Connection</h3>
									<button class="import-btn" onclick={handleTestSync} disabled={syncTestLoading || !syncUrl}>
										{#if syncTestLoading}
											<svg class="spinner-icon" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10" opacity="0.25" /><path d="M12 2a10 10 0 019.95 9" /></svg>
											Testing...
										{:else}
											<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 11.08V12a10 10 0 11-5.93-9.14" /><polyline points="22 4 12 14.01 9 11.01" /></svg>
											Test Connection
										{/if}
									</button>
									{#if syncTestMessage}
										<div class="import-result {syncTestMessage.type}">
											{#if syncTestMessage.type === 'success'}
												<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 11.08V12a10 10 0 11-5.93-9.14" /><polyline points="22 4 12 14.01 9 11.01" /></svg>
											{:else}
												<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10" /><line x1="15" y1="9" x2="9" y2="15" /><line x1="9" y1="9" x2="15" y2="15" /></svg>
											{/if}
											<span>{syncTestMessage.text}</span>
										</div>
									{/if}
								</div>

								<div class="settings-section">
									<h3>Sync</h3>
									<button class="import-btn" onclick={handleSyncNow} disabled={syncRunning || !syncUrl}>
										{#if syncRunning}
											<svg class="spinner-icon" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10" opacity="0.25" /><path d="M12 2a10 10 0 019.95 9" /></svg>
											Syncing...
										{:else}
											<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 2v6h-6"/><path d="M3 12a9 9 0 0115-6.7L21 8"/><path d="M3 22v-6h6"/><path d="M21 12a9 9 0 01-15 6.7L3 16"/></svg>
											Sync Now
										{/if}
									</button>
									{#if syncMessage}
										<div class="import-result {syncMessage.type}">
											{#if syncMessage.type === 'success'}
												<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 11.08V12a10 10 0 11-5.93-9.14" /><polyline points="22 4 12 14.01 9 11.01" /></svg>
											{:else}
												<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10" /><line x1="15" y1="9" x2="9" y2="15" /><line x1="9" y1="9" x2="15" y2="15" /></svg>
											{/if}
											<span>{syncMessage.text}</span>
										</div>
									{/if}
									{#if activeVaultConfig($appConfig)?.last_sync_time}
										<p class="setting-hint">Last sync: {new Date(activeVaultConfig($appConfig)!.last_sync_time!).toLocaleString()}</p>
									{/if}
									<p class="setting-hint">If a note is edited on two devices, both are kept (one as a "(conflict)" copy).</p>
								</div>
								<div class="settings-section">
									<h3>Automatic Sync</h3>
									<div class="setting-options">
										{#each [{ v: 0, l: 'Off' }, { v: 5, l: '5 min' }, { v: 15, l: '15 min' }, { v: 30, l: '30 min' }, { v: 60, l: '60 min' }] as opt}
											<button class="option-btn" class:active={syncIntervalMinutes === opt.v} onclick={() => { syncIntervalMinutes = opt.v; saveSyncSettings(); }}>{opt.l}</button>
										{/each}
									</div>
									<p class="setting-hint">How often to sync in the background.</p>
									<label class="setting-toggle" style="margin-top: 12px;">
										<span class="setting-label">
											<span class="setting-name">Sync when a note changes</span>
											<span class="setting-desc">Sync shortly after you edit a note.</span>
										</span>
										<button class="toggle-switch" class:on={syncOnChange} role="switch" aria-checked={syncOnChange} aria-label="Sync when a note changes" onclick={() => { syncOnChange = !syncOnChange; saveSyncSettings(); }}><span class="toggle-knob"></span></button>
									</label>
									<label class="setting-toggle">
										<span class="setting-label">
											<span class="setting-name">Sync when the vault opens</span>
											<span class="setting-desc">Sync once when the app starts.</span>
										</span>
										<button class="toggle-switch" class:on={syncOnOpen} role="switch" aria-checked={syncOnOpen} aria-label="Sync when the vault opens" onclick={() => { syncOnOpen = !syncOnOpen; saveSyncSettings(); }}><span class="toggle-knob"></span></button>
									</label>
								</div>
								<div class="settings-section">
									<h3>Folder Request Parallelism</h3>
									<div class="setting-options">
										{#each [...new Set([1, 3, 6, 12, syncFolderRequests || defaultFolderRequests])].sort((a, b) => a - b) as opt}
											<button class="option-btn" class:active={(syncFolderRequests || defaultFolderRequests) === opt} onclick={() => { syncFolderRequests = opt; saveSyncSettings(); }}>{opt}</button>
										{/each}
									</div>
									<p class="setting-hint">How many folders are listed on the server at once. 6 suits most servers; choose 1 if yours rejects concurrent requests.</p>
								</div>
							{/if}
						</div>

					{:else if activeTab === 'updates'}
						<div class="tab-content">
							<div class="settings-section">
								<h3>Current Version</h3>
								<p class="update-version">HelixNotes <strong>v{appVersion}</strong></p>
							</div>

							{#if isManagedInstall($installType)}
							<div class="settings-section">
								<h3>Updates</h3>
								<p class="setting-hint">HelixNotes was installed through your system package manager, which delivers updates. The app does not check for or install updates on its own.</p>
							</div>
							{:else}
							<div class="settings-section">
								<h3>Check for Updates</h3>
								<button class="import-btn" onclick={handleCheckUpdate} disabled={updateChecking || updateDownloading}>
									{#if updateChecking}
										<svg class="spinner-icon" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10" opacity="0.25" /><path d="M12 2a10 10 0 019.95 9" /></svg>
										Checking...
									{:else}
										<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
											<path d="M21 12a9 9 0 00-9-9 9.75 9.75 0 00-6.74 2.74L3 8"/><path d="M3 3v5h5"/>
										</svg>
										Check for Updates
									{/if}
								</button>
							</div>
							{/if}

							{#if updateAvailable}
								<div class="settings-section">
									<h3>Update Available</h3>
									<div class="update-info">
										<p class="update-new-version">Version <strong>{updateAvailable.version}</strong></p>
										{#if updateAvailable.date}
											<p class="update-date">{new Date(updateAvailable.date).toLocaleDateString()}</p>
										{/if}
										{#if updateAvailable.body}
											<div class="update-notes">{updateAvailable.body}</div>
										{/if}
									</div>
									{#if $installType === 'appimage' || $installType === 'windows' || $installType === 'macos'}
									<button class="update-install-btn" onclick={handleDownloadAndInstall} disabled={updateDownloading}>
										{#if updateDownloading}
											<svg class="spinner-icon" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10" opacity="0.25" /><path d="M12 2a10 10 0 019.95 9" /></svg>
											{updateProgress > 0 ? `Downloading ${updateProgress}%` : 'Downloading...'}
										{:else}
											<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
												<path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/>
											</svg>
											Download & Install
										{/if}
									</button>
									{#if updateDownloading && updateProgress > 0}
										<div class="update-progress-bar">
											<div class="update-progress-fill" style="width: {updateProgress}%"></div>
										</div>
									{/if}
								{:else if $installType === 'deb'}
									<div class="update-apt-info">
										<p>Update via your package manager:</p>
										<code>sudo apt update && sudo apt upgrade helix-notes</code>
									</div>
								{:else if $installType === 'aur'}
									<div class="update-apt-info">
										<p>Update via your AUR helper:</p>
										<code>yay -Syu helixnotes</code>
									</div>
								{:else if $installType === 'android'}
									<button class="update-install-btn" onclick={() => { openUrl('https://helixnotes.com/#download').catch(() => {}); }}>
										<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
											<path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/>
										</svg>
										Download from Website
									</button>
								{:else}
									<a class="update-install-btn" href="https://gitlab.com/ArkHost/HelixNotes/-/releases" target="_blank" rel="noopener">
										<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
											<path d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/>
										</svg>
										Download
									</a>
								{/if}
								</div>
							{/if}

							{#if updateMessage}
								<div class="import-result {updateMessage.type}">
									{#if updateMessage.type === 'success'}
										<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 11.08V12a10 10 0 11-5.93-9.14" /><polyline points="22 4 12 14.01 9 11.01" /></svg>
									{:else if updateMessage.type === 'error'}
										<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10" /><line x1="15" y1="9" x2="9" y2="15" /><line x1="9" y1="9" x2="15" y2="15" /></svg>
									{:else}
										<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10" /><line x1="12" y1="8" x2="12" y2="12" /><line x1="12" y1="16" x2="12.01" y2="16" /></svg>
									{/if}
									<span>{updateMessage.text}</span>
								</div>
							{/if}
						</div>
					{/if}
				</div>
			</div>
		</div>
	</div>
{/if}

<style>
	.settings-overlay {
		position: fixed;
		inset: 0;
		background: rgba(0, 0, 0, 0.35);
		display: flex;
		align-items: center;
		justify-content: center;
		z-index: 2000;
	}

	.settings-panel {
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 16px;
		box-shadow: var(--shadow-lg);
		width: 620px;
		height: 80vh;
		max-height: 700px;
		overflow: hidden;
		display: flex;
		flex-direction: column;
	}

	.settings-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 20px 24px 16px;
		border-bottom: 1px solid var(--border-light);
	}

	.settings-header h2 {
		font-size: 18px;
		font-weight: 600;
		color: var(--text-primary);
	}

	.close-btn {
		background: none;
		border: none;
		color: var(--text-tertiary);
		cursor: pointer;
		padding: 4px;
		border-radius: 6px;
		display: flex;
		align-items: center;
	}

	.close-btn:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.settings-content {
		display: flex;
		flex: 1;
		overflow: hidden;
	}

	.settings-tabs {
		display: flex;
		flex-direction: column;
		gap: 2px;
		padding: 16px 8px;
		border-right: 1px solid var(--border-light);
		min-width: 150px;
		background: var(--bg-secondary);
	}

	.tab-btn {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 10px 14px;
		border: none;
		border-radius: 8px;
		background: none;
		color: var(--text-secondary);
		font-size: 13px;
		font-weight: 500;
		cursor: pointer;
		transition: all 0.15s;
		text-align: left;
	}

	.tab-btn:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.tab-btn.active {
		background: var(--accent-light);
		color: var(--accent);
	}

	.settings-body {
		flex: 1;
		padding: 20px 24px 24px;
		overflow-y: auto;
	}

	.tab-content {
		animation: fadeIn 0.15s ease;
	}

	@keyframes fadeIn {
		from { opacity: 0; transform: translateY(4px); }
		to { opacity: 1; transform: translateY(0); }
	}

	.settings-section {
		margin-bottom: 24px;
	}

	.settings-section:last-child {
		margin-bottom: 0;
	}

	.settings-section h3 {
		font-size: 12px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--text-tertiary);
		margin-bottom: 12px;
	}

	.setting-hint {
		font-size: 13px;
		color: var(--text-tertiary);
		margin-top: 6px;
	}

	.setting-toggle {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 16px;
		padding: 10px 0;
		cursor: pointer;
	}

	.setting-toggle + .setting-toggle {
		border-top: 1px solid var(--border-light);
	}

	.setting-label {
		display: flex;
		flex-direction: column;
		gap: 2px;
		flex: 1;
		min-width: 0;
	}

	.setting-name {
		font-size: 13px;
		font-weight: 500;
		color: var(--text-primary);
	}

	.setting-desc {
		font-size: 11px;
		color: var(--text-tertiary);
		line-height: 1.4;
	}

	.toggle-switch {
		position: relative;
		width: 38px;
		height: 22px;
		border-radius: 11px;
		border: none;
		background: var(--border-color);
		cursor: pointer;
		flex-shrink: 0;
		transition: background 0.2s;
		padding: 0;
	}

	.toggle-switch.on {
		background: var(--accent);
	}

	.toggle-knob {
		position: absolute;
		top: 2px;
		left: 2px;
		width: 18px;
		height: 18px;
		border-radius: 50%;
		background: white;
		transition: transform 0.2s;
		box-shadow: 0 1px 3px rgba(0, 0, 0, 0.2);
	}

	.toggle-switch.on .toggle-knob {
		transform: translateX(16px);
	}

	.setting-options {
		display: flex;
		gap: 8px;
	}

	.option-btn {
		flex: 1;
		padding: 10px 12px;
		border: 1px solid var(--border-color);
		border-radius: 10px;
		background: var(--bg-secondary);
		color: var(--text-secondary);
		font-size: 13px;
		cursor: pointer;
		transition: all 0.15s;
		text-align: center;
	}

	.option-btn:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.option-btn.active {
		border-color: var(--accent);
		background: var(--accent-light);
		color: var(--accent);
		font-weight: 500;
	}

	.two-col-sections {
		display: grid;
		grid-template-columns: 1fr 1fr;
		gap: 0 16px;
	}

	.two-col-sections .settings-section {
		margin-bottom: 24px;
	}

	.dropdown-wrap {
		position: relative;
	}

	.dropdown-trigger {
		width: 100%;
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 8px 10px;
		background: var(--bg-secondary);
		border: 1px solid var(--border-color);
		border-radius: 8px;
		cursor: pointer;
		transition: border-color 0.15s;
	}

	.dropdown-trigger:hover,
	.dropdown-trigger.open {
		border-color: var(--accent);
	}

	.dropdown-trigger-label {
		flex: 1;
		font-size: 13px;
		color: var(--text-primary);
		text-align: left;
	}

	.dropdown-chevron {
		width: 14px;
		height: 14px;
		color: var(--text-tertiary);
		flex-shrink: 0;
		transition: transform 0.15s;
	}

	.dropdown-trigger.open .dropdown-chevron {
		transform: rotate(180deg);
	}

	.dropdown-list {
		position: absolute;
		top: calc(100% + 4px);
		left: 0;
		right: 0;
		background: var(--bg-secondary);
		border: 1px solid var(--border-color);
		border-radius: 8px;
		box-shadow: var(--shadow-md);
		z-index: 100;
		max-height: 240px;
		overflow-y: auto;
		padding: 4px;
	}

	.dropdown-item {
		width: 100%;
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 7px 8px;
		border-radius: 5px;
		background: none;
		border: none;
		cursor: pointer;
		transition: background 0.1s;
	}

	.dropdown-item:hover {
		background: var(--bg-hover);
	}

	.dropdown-item.active {
		background: var(--accent-light);
	}

	.dropdown-item-label {
		flex: 1;
		font-size: 13px;
		color: var(--text-primary);
		text-align: left;
	}

	.dropdown-check {
		width: 14px;
		height: 14px;
		color: var(--accent);
		flex-shrink: 0;
	}

	.dropdown-preview {
		display: flex;
		width: 36px;
		height: 22px;
		border-radius: 4px;
		overflow: hidden;
		background: var(--preview-bg);
		border: 1px solid rgba(0,0,0,0.12);
		flex-shrink: 0;
	}

	.preview-sidebar {
		width: 30%;
		background: var(--preview-sidebar);
		flex-shrink: 0;
	}

	.preview-main {
		flex: 1;
		background: var(--preview-bg);
		padding: 3px;
		display: flex;
		flex-direction: column;
		gap: 2px;
	}

	.preview-accent-bar {
		height: 3px;
		width: 60%;
		background: var(--preview-accent);
		border-radius: 2px;
	}

	.accent-dot {
		width: 18px;
		height: 18px;
		border-radius: 50%;
		flex-shrink: 0;
	}

	.dropdown-divider {
		height: 1px;
		background: var(--border-light);
		margin: 4px 0;
	}

	.custom-accent-item {
		cursor: pointer;
		position: relative;
	}

	.color-wheel-input {
		position: absolute;
		inset: 0;
		width: 100%;
		height: 100%;
		opacity: 0;
		cursor: pointer;
		border: none;
		padding: 0;
	}

	.font-size-options {
		display: flex;
		gap: 6px;
	}

	.font-size-btn {
		flex: 1;
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 2px;
		padding: 10px 4px;
		border: 1px solid var(--border-color);
		border-radius: 10px;
		background: var(--bg-secondary);
		cursor: pointer;
		transition: all 0.15s;
	}

	.font-size-btn:hover {
		background: var(--bg-hover);
		border-color: var(--text-tertiary);
	}

	.font-size-btn.active {
		border-color: var(--accent);
		background: var(--accent-light);
	}

	.font-size-label {
		font-size: 11px;
		color: var(--text-secondary);
		font-weight: 500;
	}

	.font-size-btn.active .font-size-label {
		color: var(--accent);
	}

	.font-size-value {
		font-size: 10px;
		color: var(--text-tertiary);
	}

	.font-family-options {
		display: grid;
		grid-template-columns: repeat(4, 1fr);
		gap: 8px;
	}

	.font-family-btn {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 6px;
		padding: 12px 8px;
		border: 1px solid var(--border-color);
		border-radius: 10px;
		background: var(--bg-secondary);
		cursor: pointer;
		transition: all 0.15s;
	}

	.font-family-btn:hover {
		background: var(--bg-hover);
		border-color: var(--text-tertiary);
	}

	.font-family-btn.active {
		border-color: var(--accent);
		background: var(--accent-light);
	}

	.font-family-preview {
		font-size: 20px;
		color: var(--text-primary);
		line-height: 1;
	}

	.font-family-btn.active .font-family-preview {
		color: var(--accent);
	}

	.font-family-name {
		font-size: 11px;
		color: var(--text-secondary);
	}

	.font-family-btn.active .font-family-name {
		color: var(--accent);
		font-weight: 500;
	}

	/* Import tab */
	.import-desc {
		font-size: 13px;
		color: var(--text-secondary);
		line-height: 1.5;
		margin-bottom: 8px;
	}

	.import-desc code {
		background: var(--bg-secondary);
		padding: 1px 5px;
		border-radius: 4px;
		font-size: 12px;
	}

	.import-warn {
		display: flex;
		align-items: center;
		gap: 8px;
		font-size: 13px;
		font-weight: 500;
		color: var(--danger);
		background: color-mix(in srgb, var(--danger) 10%, transparent);
		border: 1px solid color-mix(in srgb, var(--danger) 25%, transparent);
		border-radius: 8px;
		padding: 10px 12px;
		margin-bottom: 16px;
	}

	.import-btn {
		display: inline-flex;
		align-items: center;
		gap: 8px;
		padding: 10px 18px;
		border: 1px solid var(--border-color);
		border-radius: 10px;
		background: var(--bg-secondary);
		color: var(--text-primary);
		font-size: 13px;
		font-weight: 500;
		cursor: pointer;
		transition: all 0.15s;
	}

	.import-btn:hover:not(:disabled) {
		border-color: var(--accent);
		background: var(--accent-light);
		color: var(--accent);
	}

	.import-btn:disabled {
		opacity: 0.6;
		cursor: not-allowed;
	}

	.maintenance-stats {
		width: 100%;
		background: var(--bg-secondary);
		border: 1px solid var(--border-light);
		border-radius: 10px;
		padding: 4px 0;
	}

	.maintenance-stat-row {
		display: flex;
		justify-content: space-between;
		padding: 7px 16px;
	}

	.maintenance-stat-label {
		font-size: 13px;
		color: var(--text-secondary);
	}

	.maintenance-stat-value {
		font-size: 13px;
		font-weight: 600;
		color: var(--text-primary);
	}

	.maintenance-stat-total {
		border-top: 1px solid var(--border-light);
	}

	.maintenance-stat-total .maintenance-stat-value {
		color: var(--accent);
	}

	.vault-maintenance {
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: 10px;
	}

	.cleanup-action {
		min-height: 44px;
	}

	.cleanup-message {
		margin: 0;
		font-size: 13px;
		color: var(--text-secondary);
	}

	.cleanup-list {
		width: 100%;
		max-height: 140px;
		overflow-y: auto;
		background: var(--bg-secondary);
		border: 1px solid var(--border-light);
		border-radius: 8px;
	}

	.cleanup-row {
		display: flex;
		justify-content: space-between;
		gap: 12px;
		padding: 5px 12px;
		font-size: 12px;
	}

	.cleanup-name {
		color: var(--text-secondary);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.cleanup-size {
		color: var(--text-tertiary);
		flex-shrink: 0;
	}

	.cleanup-hint {
		margin: 0;
		font-size: 11px;
		color: var(--text-tertiary);
	}

	.spinner-icon {
		animation: spin 0.8s linear infinite;
	}

	@keyframes spin {
		from { transform: rotate(0deg); }
		to { transform: rotate(360deg); }
	}

	.import-result {
		display: flex;
		align-items: flex-start;
		gap: 8px;
		margin-top: 14px;
		padding: 10px 14px;
		border-radius: 8px;
		font-size: 13px;
	}

	.import-result.success {
		background: color-mix(in srgb, #059669 10%, transparent);
		color: #059669;
	}

	.import-stats ul {
		margin: 4px 0 0 0;
		padding-left: 16px;
		line-height: 1.6;
	}

	.import-result.error {
		background: color-mix(in srgb, #e11d48 10%, transparent);
		color: #e11d48;
	}

	/* Backup tab */
	.backup-setting-group {
		margin: 12px 0;
	}

	.backup-setting-label {
		display: block;
		font-size: 13px;
		color: var(--text-primary);
		margin-bottom: 8px;
	}

	.backup-actions {
		margin-top: 12px;
		display: flex;
		flex-direction: column;
		gap: 6px;
	}

	.backup-link-btn {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		background: none;
		border: none;
		color: var(--accent);
		font-size: 13px;
		cursor: pointer;
		padding: 4px 0;
	}

	.backup-link-btn:hover:not(:disabled) {
		text-decoration: underline;
	}

	.backup-link-btn:disabled {
		opacity: 0.6;
		cursor: not-allowed;
	}

	.backup-path {
		font-size: 11px;
		color: var(--text-tertiary);
		padding-left: 20px;
		word-break: break-all;
	}

	.backup-last-time {
		margin-top: 12px;
		font-size: 12px;
		font-style: italic;
		color: var(--text-secondary);
	}

	.backup-empty {
		font-size: 13px;
		color: var(--text-tertiary);
		padding: 12px 0;
	}

	.backup-list {
		display: flex;
		flex-direction: column;
		gap: 2px;
		max-height: 200px;
		overflow-y: auto;
	}

	.backup-item {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 8px 10px;
		border-radius: 6px;
		background: var(--bg-secondary);
	}

	.backup-info {
		display: flex;
		flex-direction: column;
		gap: 2px;
	}

	.backup-date {
		font-size: 13px;
		color: var(--text-primary);
	}

	.backup-size {
		font-size: 11px;
		color: var(--text-tertiary);
	}

	.backup-item-actions {
		display: flex;
		gap: 4px;
	}

	.backup-action-btn {
		background: none;
		border: none;
		color: var(--text-tertiary);
		cursor: pointer;
		padding: 4px;
		border-radius: 4px;
		display: flex;
		align-items: center;
	}

	.backup-action-btn:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.backup-action-btn.danger:hover {
		color: var(--danger);
		background: color-mix(in srgb, var(--danger) 10%, transparent);
	}

	.restore-confirm-overlay {
		position: fixed;
		inset: 0;
		background: rgba(0, 0, 0, 0.4);
		display: flex;
		align-items: center;
		justify-content: center;
		z-index: 3000;
	}

	.restore-confirm {
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 12px;
		box-shadow: var(--shadow-lg);
		padding: 24px;
		max-width: 380px;
	}

	.restore-confirm h4 {
		font-size: 16px;
		font-weight: 600;
		color: var(--text-primary);
		margin-bottom: 8px;
	}

	.restore-confirm p {
		font-size: 13px;
		color: var(--text-secondary);
		line-height: 1.5;
		margin-bottom: 16px;
	}

	.restore-confirm-actions {
		display: flex;
		gap: 8px;
		justify-content: flex-end;
	}

	.restore-cancel {
		background: var(--bg-secondary);
		border: 1px solid var(--border-color);
		color: var(--text-primary);
		padding: 6px 16px;
		border-radius: 6px;
		font-size: 13px;
		cursor: pointer;
	}

	.restore-cancel:hover {
		background: var(--bg-hover);
	}

	.restore-confirm-btn {
		background: var(--danger);
		border: none;
		color: white;
		padding: 6px 16px;
		border-radius: 6px;
		font-size: 13px;
		cursor: pointer;
	}

	.restore-confirm-btn:hover {
		opacity: 0.9;
	}

	/* AI tab */
	.ai-key-row {
		display: flex;
		gap: 6px;
	}

	.ai-key-input {
		flex: 1;
		padding: 8px 12px;
		border: 1px solid var(--border-color);
		border-radius: 8px;
		background: var(--bg-secondary);
		color: var(--text-primary);
		font-size: 13px;
		font-family: monospace;
		outline: none;
	}

	.ai-key-input:focus {
		border-color: var(--accent);
	}

	.ai-key-input::placeholder {
		color: var(--text-tertiary);
	}

	.ai-style-input {
		width: 100%;
		padding: 8px 12px;
		border: 1px solid var(--border-color);
		border-radius: 8px;
		background: var(--bg-secondary);
		color: var(--text-primary);
		font-size: 13px;
		font-family: inherit;
		outline: none;
		resize: vertical;
		min-height: 60px;
	}

	.ai-style-input:focus {
		border-color: var(--accent);
	}

	.ai-style-input::placeholder {
		color: var(--text-tertiary);
	}

	.ai-key-toggle {
		background: var(--bg-secondary);
		border: 1px solid var(--border-color);
		border-radius: 8px;
		padding: 8px;
		cursor: pointer;
		color: var(--text-tertiary);
		display: flex;
		align-items: center;
	}

	.ai-key-toggle:hover {
		color: var(--text-primary);
		background: var(--bg-hover);
	}

	.ai-link {
		color: var(--accent);
		text-decoration: none;
	}

	.ai-link:hover {
		text-decoration: underline;
	}

	.ai-model-options {
		display: flex;
		gap: 8px;
	}

	.ai-model-btn {
		flex: 1;
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 4px;
		padding: 12px 8px;
		border: 1px solid var(--border-color);
		border-radius: 10px;
		background: var(--bg-secondary);
		cursor: pointer;
		transition: all 0.15s;
	}

	.ai-model-btn:hover {
		background: var(--bg-hover);
		border-color: var(--text-tertiary);
	}

	.ai-model-btn.active {
		border-color: var(--accent);
		background: var(--accent-light);
	}

	.ai-model-name {
		font-size: 13px;
		font-weight: 500;
		color: var(--text-primary);
	}

	.ai-model-btn.active .ai-model-name {
		color: var(--accent);
	}

	.ai-model-desc {
		font-size: 11px;
		color: var(--text-tertiary);
	}

	/* Updates tab */
	.update-version {
		font-size: 14px;
		color: var(--text-primary);
	}

	.update-info {
		background: var(--bg-secondary);
		border: 1px solid var(--border-color);
		border-radius: 8px;
		padding: 12px 14px;
		margin-bottom: 12px;
	}

	.update-new-version {
		font-size: 14px;
		color: var(--text-primary);
		margin-bottom: 4px;
	}

	.update-date {
		font-size: 12px;
		color: var(--text-tertiary);
		margin-bottom: 8px;
	}

	.update-notes {
		font-size: 13px;
		color: var(--text-secondary);
		line-height: 1.5;
		white-space: pre-wrap;
		max-height: 150px;
		overflow-y: auto;
	}

	.update-install-btn {
		display: inline-flex;
		align-items: center;
		gap: 8px;
		padding: 10px 18px;
		border: none;
		border-radius: 10px;
		background: var(--accent);
		color: white;
		font-size: 13px;
		font-weight: 500;
		cursor: pointer;
		transition: all 0.15s;
	}

	.update-install-btn:hover:not(:disabled) {
		background: var(--accent-hover);
	}

	.update-install-btn:disabled {
		opacity: 0.6;
		cursor: not-allowed;
	}

	.update-apt-info {
		margin-top: 8px;
	}
	.update-apt-info p {
		margin: 0 0 6px 0;
		font-size: 13px;
		color: var(--text-secondary);
	}
	.update-apt-info code {
		display: block;
		padding: 8px 12px;
		background: var(--bg-secondary);
		border-radius: 6px;
		font-size: 12px;
		user-select: all;
	}

	.update-progress-bar {
		margin-top: 10px;
		height: 6px;
		border-radius: 3px;
		background: var(--bg-secondary);
		overflow: hidden;
	}

	.update-progress-fill {
		height: 100%;
		border-radius: 3px;
		background: var(--accent);
		transition: width 0.3s ease;
	}

	.import-result.info {
		background: color-mix(in srgb, var(--accent) 10%, transparent);
		color: var(--accent);
	}

	/* ═══ Mobile ═══ */

	.settings-overlay.mobile {
		align-items: stretch;
		justify-content: stretch;
	}

	.settings-panel.mobile {
		width: 100%;
		height: 100%;
		max-height: 100%;
		border-radius: 0;
		border: none;
	}

	.settings-panel.mobile .settings-content {
		flex-direction: column;
	}

	.settings-panel.mobile .settings-tabs {
		flex-direction: row;
		min-width: 0;
		border-right: none;
		border-bottom: 1px solid var(--border-light);
		padding: 8px 8px 0;
		overflow-x: auto;
		overflow-y: hidden;
		-webkit-overflow-scrolling: touch;
		scrollbar-width: none;
		gap: 0;
	}

	.settings-panel.mobile .settings-tabs::-webkit-scrollbar {
		display: none;
	}

	.settings-panel.mobile .tab-btn {
		flex-shrink: 0;
		flex-direction: column;
		gap: 4px;
		padding: 8px 14px 10px;
		font-size: 11px;
		border-radius: 8px 8px 0 0;
	}

	.settings-panel.mobile .tab-btn.active {
		border-bottom: 2px solid var(--accent);
		border-radius: 8px 8px 0 0;
	}

	.settings-panel.mobile .settings-header {
		padding-top: calc(env(safe-area-inset-top, 12px) + 12px);
	}

	.settings-panel.mobile .settings-body {
		padding: 16px;
	}

	.settings-panel.mobile .setting-name {
		font-size: 14px;
	}

	.settings-panel.mobile .setting-desc {
		font-size: 12px;
	}

	.settings-panel.mobile .toggle-switch {
		width: 44px;
		height: 26px;
	}

	.settings-panel.mobile .toggle-knob {
		width: 22px;
		height: 22px;
	}

	.settings-panel.mobile .toggle-switch.on .toggle-knob {
		transform: translateX(18px);
	}

	.settings-panel.mobile .setting-options {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(min(120px, 100%), 1fr));
	}

	.settings-panel.mobile .option-btn {
		min-width: 0;
		padding: 12px 10px;
		font-size: 14px;
	}

	.settings-panel.mobile .theme-btn {
		padding: 12px 10px;
		font-size: 14px;
	}

	.settings-panel.mobile .font-family-options,
	.settings-panel.mobile .font-size-options {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(85px, 1fr));
		gap: 8px;
	}

	.settings-panel.mobile .font-size-btn {
		padding: 12px 6px;
	}

	@media (max-width: 480px) {
		.settings-panel.mobile .two-col-sections {
			grid-template-columns: minmax(0, 1fr);
		}
	}

	/* ── Custom Themes ── */
	.section-header-row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		margin-bottom: 12px;
	}

	.section-header-row h3 {
		margin: 0;
	}

	.create-theme-btn {
		display: flex;
		align-items: center;
		gap: 5px;
		padding: 5px 10px;
		background: var(--accent-light);
		color: var(--text-accent);
		border: 1px solid transparent;
		border-radius: 6px;
		font-size: 12px;
		font-weight: 500;
		cursor: pointer;
		transition: background 0.15s;
	}

	.create-theme-btn:hover {
		background: var(--accent);
		color: #fff;
	}

	.custom-theme-list {
		display: flex;
		flex-direction: column;
		gap: 4px;
		margin-bottom: 10px;
	}

	.custom-theme-row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		border-radius: 8px;
		border: 1px solid transparent;
		padding: 2px 4px 2px 0;
	}

	.custom-theme-row.active {
		background: var(--accent-light);
		border-color: var(--accent);
	}

	.custom-theme-swatch-btn {
		display: flex;
		align-items: center;
		gap: 9px;
		flex: 1;
		background: none;
		border: none;
		cursor: pointer;
		padding: 5px 6px;
		border-radius: 6px;
		color: var(--text-primary);
		font-size: 13px;
		text-align: left;
	}

	.custom-theme-swatch-btn:hover {
		background: var(--bg-hover);
	}

	.custom-theme-swatch {
		width: 32px;
		height: 22px;
		border-radius: 5px;
		border: 1px solid;
		display: flex;
		overflow: hidden;
		flex-shrink: 0;
	}

	.swatch-sidebar {
		width: 35%;
		height: 100%;
	}

	.custom-theme-name {
		flex: 1;
	}

	.custom-theme-actions {
		display: flex;
		gap: 2px;
	}

	.icon-btn {
		background: none;
		border: none;
		color: var(--text-secondary);
		cursor: pointer;
		padding: 5px;
		border-radius: 5px;
		display: flex;
		align-items: center;
	}

	.icon-btn:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.icon-btn.danger:hover {
		background: rgba(220, 38, 38, 0.1);
		color: var(--danger);
	}

	.custom-theme-io {
		display: flex;
		gap: 8px;
		margin-top: 10px;
		flex-wrap: wrap;
	}

	/* ── Custom Theme Modal ── */
	.custom-theme-modal-overlay {
		position: fixed;
		inset: 0;
		background: rgba(0, 0, 0, 0.5);
		display: flex;
		align-items: center;
		justify-content: center;
		z-index: 3000;
	}

	.custom-theme-modal {
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 14px;
		box-shadow: var(--shadow-lg);
		width: 480px;
		max-height: 85vh;
		display: flex;
		flex-direction: column;
		overflow: hidden;
	}

	.custom-theme-modal-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 18px 20px 14px;
		border-bottom: 1px solid var(--border-light);
	}

	.custom-theme-modal-header h3 {
		font-size: 15px;
		font-weight: 600;
		color: var(--text-primary);
		margin: 0;
	}

	.custom-theme-modal-body {
		overflow-y: auto;
		padding: 16px 20px;
		display: flex;
		flex-direction: column;
		gap: 16px;
	}

	.custom-theme-modal-footer {
		display: flex;
		justify-content: flex-end;
		gap: 8px;
		padding: 14px 20px;
		border-top: 1px solid var(--border-light);
	}

	.ct-field {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}

	.ct-label {
		font-size: 12px;
		font-weight: 500;
		color: var(--text-secondary);
		text-transform: uppercase;
		letter-spacing: 0.04em;
	}

	.ct-name-input {
		background: var(--bg-secondary);
		border: 1px solid var(--border-color);
		border-radius: 7px;
		color: var(--text-primary);
		font-size: 14px;
		padding: 8px 10px;
		width: 100%;
		box-sizing: border-box;
		outline: none;
	}

	.ct-name-input:focus {
		border-color: var(--accent);
	}

	.ct-colors-grid {
		display: grid;
		grid-template-columns: 1fr 1fr;
		gap: 16px;
	}

	.ct-color-group {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}

	.ct-group-label {
		font-size: 11px;
		font-weight: 600;
		color: var(--text-secondary);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		margin-bottom: 2px;
	}

	.ct-color-row {
		display: flex;
		align-items: center;
		gap: 8px;
		cursor: pointer;
	}

	.ct-color-input {
		width: 30px;
		height: 24px;
		border: 1px solid var(--border-color);
		border-radius: 5px;
		padding: 1px;
		cursor: pointer;
		background: none;
		flex-shrink: 0;
	}

	.ct-color-label {
		font-size: 13px;
		color: var(--text-primary);
	}

	/* Theme preview */
	.ct-preview {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}

	.ct-preview-label {
		font-size: 11px;
		font-weight: 600;
		color: var(--text-secondary);
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}

	.ct-preview-window {
		display: flex;
		border-radius: 8px;
		overflow: hidden;
		border: 1px solid var(--p-border);
		height: 80px;
		background: var(--p-bg);
	}

	.ct-preview-sidebar {
		width: 80px;
		background: var(--p-sidebar);
		border-right: 1px solid var(--p-border);
		padding: 8px 6px;
		display: flex;
		flex-direction: column;
		gap: 4px;
	}

	.ct-preview-item {
		height: 10px;
		border-radius: 3px;
		background: var(--p-text2);
		opacity: 0.4;
	}

	.ct-preview-item.active {
		background: var(--p-hover);
		opacity: 1;
	}

	.ct-preview-main {
		flex: 1;
		padding: 10px 12px;
		display: flex;
		flex-direction: column;
		gap: 5px;
	}

	.ct-preview-title {
		height: 10px;
		border-radius: 3px;
		background: var(--p-text);
		width: 60%;
		opacity: 0.8;
	}

	.ct-preview-line {
		height: 7px;
		border-radius: 3px;
		background: var(--p-text2);
		opacity: 0.5;
	}

	.ct-preview-line.long { width: 90%; }
	.ct-preview-line.medium { width: 70%; }
	.ct-preview-line.short { width: 45%; }

	.cancel-btn {
		padding: 7px 14px;
		background: var(--bg-secondary);
		border: 1px solid var(--border-color);
		border-radius: 7px;
		color: var(--text-primary);
		font-size: 13px;
		cursor: pointer;
	}

	.cancel-btn:hover {
		background: var(--bg-hover);
	}

	.save-btn {
		padding: 7px 16px;
		background: var(--accent);
		border: none;
		border-radius: 7px;
		color: #fff;
		font-size: 13px;
		font-weight: 500;
		cursor: pointer;
	}

	.save-btn:hover:not(:disabled) {
		opacity: 0.9;
	}

	.save-btn:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}

</style>
