<script>
	import { invoke } from '@tauri-apps/api/core';
	import { open as openDialog } from '@tauri-apps/plugin-dialog';
	import { onMount } from 'svelte';

	let watchDir = $state('');
	let apiUrl = $state('');
	let autostart = $state(false);
	let startMinimized = $state(false);
	let autostartLoaded = $state(false);
	let autostartError = $state('');
	let saved = $state(false);
	let loaded = $state(false);

	onMount(async () => {
		const config = await invoke('get_config');
		watchDir = config.watchDir;
		apiUrl = config.apiUrl;
		autostart = config.autostart;
		startMinimized = config.startMinimized ?? false;
		loaded = true;

		try {
			autostart = await invoke('is_autostart_enabled');
		} catch (e) {
			autostartError = `Failed to check autostart: ${e}`;
		}
		autostartLoaded = true;
	});

	async function browseWatchDir() {
		const selected = await openDialog({
			directory: true,
			defaultPath: watchDir || undefined,
			title: 'Select watch folder',
		});
		if (selected) {
			watchDir = selected;
		}
	}

	function revealWatchDir() {
		if (watchDir) {
			invoke('reveal_path', { path: watchDir });
		}
	}

	async function handleSave() {
		autostartError = '';
		try {
			if (autostart) {
				await invoke('enable_autostart');
			} else {
				await invoke('disable_autostart');
			}
		} catch (e) {
			autostartError = `Failed to update autostart: ${e}`;
		}

		const newConfig = { watchDir, apiUrl, autostart, startMinimized };
		await invoke('save_config_cmd', { config: newConfig });

		saved = true;
		setTimeout(() => { saved = false; }, 2000);
	}
</script>

<div class="flex flex-col h-screen bg-zinc-900 text-zinc-200">
	<div class="px-3 py-2.5 border-b border-zinc-800">
		<h1 class="text-sm font-semibold">Settings</h1>
	</div>

	{#if loaded}
		<div class="flex-1 overflow-y-auto px-3 py-3">
			<div class="space-y-4">
				<div>
					<label for="watch-dir" class="block text-[11px] uppercase tracking-wider text-zinc-500 mb-1.5">
						Watch folder
					</label>
					<div class="flex gap-1.5">
						<input
							id="watch-dir"
							type="text"
							bind:value={watchDir}
							class="flex-1 min-w-0 rounded-md border border-zinc-700 bg-zinc-800 px-2.5 py-1.5 text-sm text-zinc-200 outline-none focus:border-blue-500 transition-colors"
						/>
						<button
							onclick={browseWatchDir}
							title="Browse"
							class="rounded-md border border-zinc-700 bg-zinc-800 px-2 py-1.5 text-sm text-zinc-400 hover:text-zinc-200 hover:border-zinc-600 transition-colors"
						>
							<svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
								<path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
							</svg>
						</button>
						<button
							onclick={revealWatchDir}
							disabled={!watchDir}
							title="Reveal in file manager"
							class="rounded-md border border-zinc-700 bg-zinc-800 px-2 py-1.5 text-sm text-zinc-400 hover:text-zinc-200 hover:border-zinc-600 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
						>
							<svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
								<path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
							</svg>
						</button>
					</div>
				</div>

				<div>
					<label for="api-url" class="block text-[11px] uppercase tracking-wider text-zinc-500 mb-1.5">
						API URL
					</label>
					<input
						id="api-url"
						type="text"
						bind:value={apiUrl}
						class="w-full rounded-md border border-zinc-700 bg-zinc-800 px-2.5 py-1.5 text-sm text-zinc-200 outline-none focus:border-blue-500 transition-colors"
					/>
				</div>

				<div>
					<div class="flex items-center justify-between">
						<label for="autostart" class="text-sm text-zinc-300">Launch at login</label>
						<button
							id="autostart"
							role="switch"
							aria-checked={autostart}
							disabled={!autostartLoaded}
							onclick={() => { autostart = !autostart; }}
							class="relative inline-flex h-5 w-9 items-center rounded-full transition-colors {autostart ? 'bg-blue-500' : 'bg-zinc-600'} {!autostartLoaded ? 'opacity-50 cursor-not-allowed' : ''}"
						>
							<span class="inline-block h-3.5 w-3.5 rounded-full bg-white transition-transform {autostart ? 'translate-x-4' : 'translate-x-0.5'}" />
						</button>
					</div>
					{#if autostartError}
						<p class="text-xs text-red-400 mt-1">{autostartError}</p>
					{/if}
				</div>

				<div>
					<div class="flex items-center justify-between">
						<label for="start-minimized" class="text-sm text-zinc-300">Start minimized</label>
						<button
							id="start-minimized"
							role="switch"
							aria-checked={startMinimized}
							onclick={() => { startMinimized = !startMinimized; }}
							class="relative inline-flex h-5 w-9 items-center rounded-full transition-colors {startMinimized ? 'bg-blue-500' : 'bg-zinc-600'}"
						>
							<span class="inline-block h-3.5 w-3.5 rounded-full bg-white transition-transform {startMinimized ? 'translate-x-4' : 'translate-x-0.5'}" />
						</button>
					</div>
				</div>

				<button
					onclick={handleSave}
					class="w-full rounded-md bg-blue-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-blue-500 transition-colors"
				>
					{saved ? 'Saved!' : 'Save'}
				</button>
			</div>
		</div>
	{/if}
</div>
