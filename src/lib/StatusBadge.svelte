<script>
	let { status, retryCount = 0 } = $props();

	const config = $derived.by(() => {
		switch (status) {
			case 'pending':
				return { label: 'Pending', bg: 'bg-zinc-700', text: 'text-zinc-300' };
			case 'uploading':
				return { label: 'Uploading', bg: 'bg-blue-900/50', text: 'text-blue-400' };
			case 'queued':
				return { label: 'Queued', bg: 'bg-green-900/50', text: 'text-green-400' };
			case 'duplicate':
				return { label: 'Duplicate', bg: 'bg-zinc-700', text: 'text-zinc-400' };
			case 'error':
				if (retryCount < 5) {
					return { label: 'Uploading', bg: 'bg-blue-900/50', text: 'text-blue-400' };
				}
				return { label: 'Failed', bg: 'bg-red-900/50', text: 'text-red-400' };
			default:
				return { label: status, bg: 'bg-zinc-700', text: 'text-zinc-300' };
		}
	});
</script>

<span class="inline-flex items-center rounded-full px-2 py-0.5 text-[11px] font-medium {config.bg} {config.text}">
	{#if status === 'uploading' || (status === 'error' && retryCount < 5)}
		<span class="mr-1 h-1.5 w-1.5 rounded-full bg-blue-400 animate-pulse"></span>
	{/if}
	{config.label}
</span>
