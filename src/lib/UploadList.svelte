<script>
	import StatusBadge from './StatusBadge.svelte';

	let { uploads = [] } = $props();

	function timeAgo(dateStr) {
		const now = Date.now();
		const then = new Date(dateStr).getTime();
		const seconds = Math.floor((now - then) / 1000);

		if (seconds < 60) return 'just now';
		const minutes = Math.floor(seconds / 60);
		if (minutes < 60) return `${minutes}m ago`;
		const hours = Math.floor(minutes / 60);
		if (hours < 24) return `${hours}h ago`;
		const days = Math.floor(hours / 24);
		return `${days}d ago`;
	}

	function truncate(str, len = 30) {
		if (str.length <= len) return str;
		return str.slice(0, len - 1) + '\u2026';
	}
</script>

<div class="flex-1 overflow-y-auto px-3 pb-3">
	{#if uploads.length === 0}
		<div class="flex flex-col items-center justify-center h-full text-zinc-500">
			<svg class="w-8 h-8 mb-2 opacity-50" fill="none" stroke="currentColor" viewBox="0 0 24 24">
				<path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
					d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
				<path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
					d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
			</svg>
			<p class="text-sm">Watching for new replays&hellip;</p>
		</div>
	{:else}
		<div class="space-y-1">
			{#each uploads as upload (upload.id)}
				<div class="flex items-center justify-between rounded-md px-2.5 py-2 hover:bg-zinc-800/50 transition-colors">
					<div class="min-w-0 flex-1 mr-2">
						<p class="text-sm text-zinc-200 truncate" title={upload.fileName}>
							{truncate(upload.fileName, 32)}
						</p>
						<p class="text-[11px] text-zinc-500 mt-0.5">
							{timeAgo(upload.createdAt)}
						</p>
						{#if upload.status === 'error' && upload.retryCount >= 5 && upload.error}
							<p class="text-[11px] text-red-400/80 mt-0.5 truncate" title={upload.error}>
								{upload.error}
							</p>
						{/if}
					</div>
					<StatusBadge status={upload.status} retryCount={upload.retryCount} />
				</div>
			{/each}
		</div>
	{/if}
</div>
