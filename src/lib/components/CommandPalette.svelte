<script lang="ts">
	import { showCommandPalette, showSearch, theme, sourceMode, viewMode, activeNotebook, activeTag } from '$lib/stores/app';
	import { setTheme, reindex } from '$lib/api';
	import { darkThemes } from '$lib/platform';

	// onNavigate runs the parent's view-change handler (refreshes the note list and reveals it
	// if it was hidden), so opening a view here behaves exactly like clicking it in the sidebar.
	let { onNavigate = () => {} }: { onNavigate?: () => void } = $props();

	function openView(mode: 'all' | 'quickaccess' | 'tasks' | 'daily' | 'trash') {
		$viewMode = mode;
		$activeNotebook = null;
		$activeTag = null;
		$showCommandPalette = false;
		onNavigate();
	}

	interface Command {
		id: string;
		label: string;
		shortcut?: string;
		action: () => void;
	}

	const modKey = navigator.platform.startsWith('Mac') ? '⌘' : 'Ctrl';

	let query = $state('');
	let selectedIndex = $state(0);
	let inputEl = $state<HTMLInputElement>(null!);

	const commands: Command[] = [
		{
			id: 'search',
			label: 'Search Notes',
			shortcut: `${modKey}+F`,
			action: () => {
				$showCommandPalette = false;
				$showSearch = true;
			}
		},
		{
			id: 'open-all-notes',
			label: 'Open All Notes',
			action: () => openView('all')
		},
		{
			id: 'open-quick-access',
			label: 'Open Quick Access',
			action: () => openView('quickaccess')
		},
		{
			id: 'open-tasks',
			label: 'Open Tasks',
			action: () => openView('tasks')
		},
		{
			id: 'open-daily',
			label: 'Open Daily Notes',
			action: () => openView('daily')
		},
		{
			id: 'open-trash',
			label: 'Open Trash (restore deleted notes)',
			action: () => openView('trash')
		},
		{
			id: 'theme-light',
			label: 'Switch to Light Theme',
			action: () => {
				$theme = 'light';
				setTheme('light');
				applyTheme('light');
				$showCommandPalette = false;
			}
		},
		{
			id: 'theme-dark',
			label: 'Switch to Dark Theme',
			action: () => {
				$theme = 'dark';
				setTheme('dark');
				applyTheme('dark');
				$showCommandPalette = false;
			}
		},
		{
			id: 'theme-system',
			label: 'Use System Theme',
			action: () => {
				$theme = 'system';
				setTheme('system');
				$showCommandPalette = false;
			}
		},
		{
			id: 'theme-solarized-light',
			label: 'Switch to Solarized Light Theme',
			action: () => { $theme = 'solarized-light'; setTheme('solarized-light'); applyTheme('solarized-light'); $showCommandPalette = false; }
		},
		{
			id: 'theme-solarized-dark',
			label: 'Switch to Solarized Dark Theme',
			action: () => { $theme = 'solarized-dark'; setTheme('solarized-dark'); applyTheme('solarized-dark'); $showCommandPalette = false; }
		},
		{
			id: 'theme-catppuccin',
			label: 'Switch to Catppuccin Theme',
			action: () => { $theme = 'catppuccin'; setTheme('catppuccin'); applyTheme('catppuccin'); $showCommandPalette = false; }
		},
		{
			id: 'theme-nord',
			label: 'Switch to Nord Theme',
			action: () => { $theme = 'nord'; setTheme('nord'); applyTheme('nord'); $showCommandPalette = false; }
		},
		{
			id: 'theme-tokyo-night',
			label: 'Switch to Tokyo Night Theme',
			action: () => { $theme = 'tokyo-night'; setTheme('tokyo-night'); applyTheme('tokyo-night'); $showCommandPalette = false; }
		},
		{
			id: 'theme-github-light',
			label: 'Switch to GitHub Light Theme',
			action: () => { $theme = 'github-light'; setTheme('github-light'); applyTheme('github-light'); $showCommandPalette = false; }
		},
		{
			id: 'theme-github-dark',
			label: 'Switch to GitHub Dark Theme',
			action: () => { $theme = 'github-dark'; setTheme('github-dark'); applyTheme('github-dark'); $showCommandPalette = false; }
		},
		{
			id: 'theme-dracula',
			label: 'Switch to Dracula Theme',
			action: () => { $theme = 'dracula'; setTheme('dracula'); applyTheme('dracula'); $showCommandPalette = false; }
		},
		{
			id: 'toggle-source',
			label: 'Toggle Source/WYSIWYG Mode',
			action: () => {
				$sourceMode = !$sourceMode;
				$showCommandPalette = false;
			}
		},
		{
			id: 'reindex',
			label: 'Rebuild Search Index',
			action: async () => {
				await reindex();
				$showCommandPalette = false;
			}
		}
	];

	let filteredCommands = $derived(
		query.trim()
			? commands.filter((c) => c.label.toLowerCase().includes(query.toLowerCase()))
			: commands
	);

	$effect(() => {
		if ($showCommandPalette && inputEl) {
			query = '';
			selectedIndex = 0;
			setTimeout(() => inputEl?.focus(), 50);
		}
	});

	function handleEscape(e: KeyboardEvent) {
		if (e.key !== 'Escape') return;
		e.preventDefault();
		e.stopPropagation();
		$showCommandPalette = false;
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'ArrowDown') {
			e.preventDefault();
			selectedIndex = Math.min(selectedIndex + 1, filteredCommands.length - 1);
		} else if (e.key === 'ArrowUp') {
			e.preventDefault();
			selectedIndex = Math.max(selectedIndex - 1, 0);
		} else if (e.key === 'Enter' && filteredCommands.length > 0) {
			filteredCommands[selectedIndex].action();
		}
	}

	function applyTheme(t: string) {
		const namedThemes = ['solarized-light', 'solarized-dark', 'catppuccin', 'nord', 'tokyo-night', 'github-light', 'github-dark', 'dracula', 'blueberry', 'forest-green', 'gruvbox', 'midnight-tide', 'cherry-blossom', 'synthwave', 'ember', 'moonlit', 'light-coffee', 'dark-coffee', 'cotton-candy', 'crimson', 'cloud', 'peach', 'material-dark', 'material-light', 'monokai', 'rose-pine', 'everforest', 'horizon', 'cyberpunk', 'black', 'one-dark', 'matrix', 'amber-crt', 'sepia', 'deep-ocean', 'sandstone', 'lavender', 'graphite', 'nebula', 'wine', 'jade'];
		const root = document.documentElement;
		root.classList.remove('dark');
		root.removeAttribute('data-theme');
		if (namedThemes.includes(t)) {
			root.setAttribute('data-theme', t);
			if (darkThemes.includes(t)) root.classList.add('dark');
		} else if (t === 'dark') {
			root.classList.add('dark');
		}
	}
</script>

{#if $showCommandPalette}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="palette-overlay" onclick={() => ($showCommandPalette = false)} onkeydowncapture={handleEscape} onkeydown={handleKeydown}>
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div class="palette-panel" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()}>
			<div class="palette-input-wrapper">
				<svg width="16" height="16" viewBox="0 0 16 16" fill="var(--text-tertiary)">
					<path d="M3 2v4.586l7 7L14.586 9l-7-7H3zm2 1a1 1 0 110 2 1 1 0 010-2z" />
				</svg>
				<input
					bind:this={inputEl}
					type="text"
					placeholder="Type a command..."
					bind:value={query}
					onkeydown={handleKeydown}
				/>
			</div>

			<div class="palette-results">
				{#each filteredCommands as cmd, i (cmd.id)}
					<button
						class="cmd-item"
						class:selected={i === selectedIndex}
						onclick={() => cmd.action()}
						onmouseenter={() => (selectedIndex = i)}
					>
						<span class="cmd-label">{cmd.label}</span>
						{#if cmd.shortcut}
							<span class="cmd-shortcut">{cmd.shortcut}</span>
						{/if}
					</button>
				{/each}
			</div>
		</div>
	</div>
{/if}

<style>
	.palette-overlay {
		position: fixed;
		inset: 0;
		background: rgba(0, 0, 0, 0.3);
		display: flex;
		align-items: flex-start;
		justify-content: center;
		padding-top: 20vh;
		z-index: 2000;
	}

	.palette-panel {
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 12px;
		box-shadow: var(--shadow-lg);
		width: 480px;
		max-height: 360px;
		overflow: hidden;
		display: flex;
		flex-direction: column;
	}

	.palette-input-wrapper {
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 14px 16px;
		border-bottom: 1px solid var(--border-light);
	}

	.palette-input-wrapper input {
		flex: 1;
		border: none;
		background: none;
		color: var(--text-primary);
		font-size: 15px;
		outline: none;
	}

	.palette-input-wrapper input::placeholder {
		color: var(--text-tertiary);
	}

	.palette-results {
		overflow-y: auto;
		padding: 4px;
	}

	.cmd-item {
		display: flex;
		align-items: center;
		justify-content: space-between;
		width: 100%;
		padding: 10px 14px;
		border: none;
		background: none;
		border-radius: 8px;
		cursor: pointer;
		text-align: left;
		transition: background 0.1s;
	}

	.cmd-item:hover,
	.cmd-item.selected {
		background: var(--bg-hover);
	}

	.cmd-label {
		font-size: 14px;
		color: var(--text-primary);
	}

	.cmd-shortcut {
		font-size: 12px;
		color: var(--text-tertiary);
		background: var(--bg-tertiary);
		padding: 2px 8px;
		border-radius: 4px;
	}
</style>
