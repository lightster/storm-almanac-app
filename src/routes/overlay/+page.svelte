<script>
	import { onMount, onDestroy } from 'svelte';
	import { getCurrentWindow } from '@tauri-apps/api/window';
	import { listen } from '@tauri-apps/api/event';
	import { invoke } from '@tauri-apps/api/core';
	import { page } from '$app/state';

	let mode = $state('interactive');
	let counter = $state(0);
	/** @type {'blocking' | 'interactable'} */
	let blockerMode = $state('blocking');
	/** @type {{ hero: string, rect: {x:number,y:number,width:number,height:number}, win_rates: {overall: number|null, player: number|null, player_games: number|null} }[]} */
	let draftHeroes = $state([]);
	let flashing = $state(false);
	/** @type {(() => void) | undefined} */
	let unlisten;
	/** @type {ReturnType<typeof setTimeout> | undefined} */
	let flashTimer;

	onMount(async () => {
		const m = page.url.searchParams.get('mode');
		mode = m === 'clickthrough' || m === 'blocker' || m === 'draft' ? m : 'interactive';

		if (mode === 'clickthrough' || mode === 'draft') {
			try {
				await getCurrentWindow().setIgnoreCursorEvents(true);
			} catch (e) {
				console.error('setIgnoreCursorEvents failed', e);
			}
		}

		if (mode === 'blocker') {
			unlisten = await listen('blocker://mode-changed', (event) => {
				const next = event?.payload;
				if (next === 'blocking' || next === 'interactable') {
					blockerMode = next;
				}
			});
		}

		if (mode === 'draft') {
			try {
				const data = await invoke('get_draft_overlay_data');
				if (data && Array.isArray(data.heroes)) {
					draftHeroes = data.heroes;
				}
			} catch (e) {
				console.error('get_draft_overlay_data failed', e);
			}
		}
	});

	onDestroy(() => {
		unlisten?.();
		if (flashTimer) clearTimeout(flashTimer);
	});

	async function close() {
		try {
			await getCurrentWindow().close();
		} catch (e) {
			console.error('close failed', e);
		}
	}

	function onBlockerMouseDown() {
		if (blockerMode !== 'blocking') return;
		flashing = false;
		// Force a tick so the class re-applies and the animation re-runs.
		requestAnimationFrame(() => {
			flashing = true;
			if (flashTimer) clearTimeout(flashTimer);
			flashTimer = setTimeout(() => {
				flashing = false;
			}, 220);
		});
	}

	/** @param {'East' | 'North' | 'NorthEast' | 'NorthWest' | 'South' | 'SouthEast' | 'SouthWest' | 'West'} direction */
	async function startResize(direction) {
		try {
			await getCurrentWindow().startResizeDragging(direction);
		} catch (e) {
			console.error('startResizeDragging failed', e);
		}
	}

	/** @param {number|null} wr */
	function fmt(wr) {
		return wr === null ? '—' : wr.toFixed(0) + '%';
	}
	/** @param {number|null} wr */
	function wrClass(wr) {
		if (wr === null) return 'neutral';
		if (wr >= 55) return 'good';
		if (wr < 45) return 'bad';
		return 'neutral';
	}

	/** @param {Event} e */
	function suppressContextMenu(e) {
		// Right-click on the blocker is meant to be absorbed silently — no
		// WebView2/WebKit native context menu. Also fire the flash so the
		// user gets the same "absorbed!" cue as a left-click in blocking mode.
		// (The POC overlays keep the native context menu for dev debugging.)
		if (mode !== 'blocker') return;
		e.preventDefault();
		if (blockerMode === 'blocking') {
			onBlockerMouseDown();
		}
	}
</script>

<svelte:window oncontextmenu={suppressContextMenu} />

<svelte:head>
	<style>
		html,
		body {
			background: transparent !important;
		}
	</style>
</svelte:head>

{#if mode === 'interactive'}
	<div class="card interactive" data-tauri-drag-region>
		<div class="title" data-tauri-drag-region>INTERACTIVE</div>
		<div class="hint" data-tauri-drag-region>drag me anywhere</div>
		<div class="row">
			<button onclick={() => counter++}>clicked {counter}</button>
			<button class="close" onclick={close} aria-label="close">×</button>
		</div>
	</div>
{:else if mode === 'clickthrough'}
	<div class="card clickthrough">
		<div class="title">CLICK-THROUGH</div>
		<div class="hint">try clicking the desktop behind me</div>
	</div>
{:else if mode === 'blocker'}
	<div
		class="blocker {blockerMode} {flashing ? 'flash' : ''}"
		onmousedown={onBlockerMouseDown}
		role="presentation"
	>
		{#if blockerMode === 'interactable'}
			<!-- Drag region covers the interior; resize handles overlay the edges. -->
			<div class="blocker-drag" data-tauri-drag-region></div>
			<div class="blocker-label" data-tauri-drag-region>
				Map Blocker — drag to move, edges to resize, Ctrl+Shift+B to lock
			</div>
			<div
				class="resize handle-n"
				onmousedown={(e) => {
					e.stopPropagation();
					startResize('North');
				}}
				role="presentation"
			></div>
			<div
				class="resize handle-s"
				onmousedown={(e) => {
					e.stopPropagation();
					startResize('South');
				}}
				role="presentation"
			></div>
			<div
				class="resize handle-e"
				onmousedown={(e) => {
					e.stopPropagation();
					startResize('East');
				}}
				role="presentation"
			></div>
			<div
				class="resize handle-w"
				onmousedown={(e) => {
					e.stopPropagation();
					startResize('West');
				}}
				role="presentation"
			></div>
			<div
				class="resize handle-ne"
				onmousedown={(e) => {
					e.stopPropagation();
					startResize('NorthEast');
				}}
				role="presentation"
			></div>
			<div
				class="resize handle-nw"
				onmousedown={(e) => {
					e.stopPropagation();
					startResize('NorthWest');
				}}
				role="presentation"
			></div>
			<div
				class="resize handle-se"
				onmousedown={(e) => {
					e.stopPropagation();
					startResize('SouthEast');
				}}
				role="presentation"
			></div>
			<div
				class="resize handle-sw"
				onmousedown={(e) => {
					e.stopPropagation();
					startResize('SouthWest');
				}}
				role="presentation"
			></div>
		{/if}
	</div>
{:else if mode === 'draft'}
	{#each draftHeroes as h (h.hero + h.rect.x + h.rect.y)}
		<div
			class="draft-label"
			style="left: {h.rect.x}px; top: {h.rect.y + h.rect.height + 2}px;"
		>
			<span class="wr overall {wrClass(h.win_rates.overall)}">
				{fmt(h.win_rates.overall)}
			</span>
			{#if h.win_rates.player !== null}
				<span class="wr player {wrClass(h.win_rates.player)}">
					you {fmt(h.win_rates.player)}
				</span>
			{/if}
		</div>
	{/each}
{/if}

<style>
	:global(html),
	:global(body) {
		background: transparent !important;
		margin: 0;
		padding: 0;
		overflow: hidden;
	}

	.card {
		box-sizing: border-box;
		width: 100vw;
		height: 100vh;
		padding: 14px 16px;
		border-radius: 14px;
		font-family: 'DM Sans', system-ui, sans-serif;
		color: #fafafa;
		display: flex;
		flex-direction: column;
		gap: 8px;
		backdrop-filter: blur(12px);
		-webkit-backdrop-filter: blur(12px);
	}

	.interactive {
		background: rgba(59, 130, 246, 0.55);
		border: 1px solid rgba(147, 197, 253, 0.7);
		box-shadow: 0 8px 24px rgba(0, 0, 0, 0.35);
	}

	.clickthrough {
		background: rgba(236, 72, 153, 0.45);
		border: 1px dashed rgba(251, 207, 232, 0.8);
	}

	.title {
		font-size: 12px;
		font-weight: 700;
		letter-spacing: 0.12em;
	}

	.hint {
		font-size: 11px;
		opacity: 0.85;
	}

	.row {
		margin-top: auto;
		display: flex;
		gap: 8px;
		align-items: center;
	}

	button {
		font: inherit;
		color: #fafafa;
		background: rgba(255, 255, 255, 0.18);
		border: 1px solid rgba(255, 255, 255, 0.3);
		border-radius: 8px;
		padding: 6px 10px;
		cursor: pointer;
	}

	button:hover {
		background: rgba(255, 255, 255, 0.28);
	}

	button.close {
		margin-left: auto;
		padding: 2px 8px;
		font-size: 14px;
		line-height: 1;
	}

	.blocker {
		box-sizing: border-box;
		width: 100vw;
		height: 100vh;
		font-family: 'DM Sans', system-ui, sans-serif;
		color: #fafafa;
		display: flex;
		align-items: flex-end;
		justify-content: flex-start;
		padding: 6px 8px;
		transition: background 120ms ease-out;
	}

	.blocker.blocking {
		background: transparent;
		border: 1px dashed rgba(255, 100, 100, 0.35);
	}

	.blocker.blocking.flash {
		animation: flashPulse 220ms ease-out;
	}

	@keyframes flashPulse {
		0% {
			background: rgba(255, 80, 80, 0.18);
		}
		100% {
			background: transparent;
		}
	}

	.blocker.interactable {
		background: rgba(30, 30, 35, 0.55);
		border: 2px dashed rgba(236, 72, 153, 0.85);
		position: relative;
	}

	.blocker-drag {
		position: absolute;
		inset: 8px;
		cursor: move;
	}

	.blocker-label {
		position: relative;
		z-index: 2;
		font-size: 11px;
		font-weight: 600;
		letter-spacing: 0.06em;
		background: rgba(0, 0, 0, 0.55);
		padding: 4px 8px;
		border-radius: 6px;
	}

	/* Resize handles — narrow strips on edges, small squares on corners.
	   z-index above the drag region so they win the hit-test on overlap. */
	.resize {
		position: absolute;
		z-index: 3;
	}
	.handle-n {
		top: 0;
		left: 8px;
		right: 8px;
		height: 6px;
		cursor: n-resize;
	}
	.handle-s {
		bottom: 0;
		left: 8px;
		right: 8px;
		height: 6px;
		cursor: s-resize;
	}
	.handle-e {
		top: 8px;
		bottom: 8px;
		right: 0;
		width: 6px;
		cursor: e-resize;
	}
	.handle-w {
		top: 8px;
		bottom: 8px;
		left: 0;
		width: 6px;
		cursor: w-resize;
	}
	.handle-ne {
		top: 0;
		right: 0;
		width: 10px;
		height: 10px;
		cursor: ne-resize;
	}
	.handle-nw {
		top: 0;
		left: 0;
		width: 10px;
		height: 10px;
		cursor: nw-resize;
	}
	.handle-se {
		bottom: 0;
		right: 0;
		width: 10px;
		height: 10px;
		cursor: se-resize;
	}
	.handle-sw {
		bottom: 0;
		left: 0;
		width: 10px;
		height: 10px;
		cursor: sw-resize;
	}

	.draft-label {
		position: absolute;
		display: flex;
		gap: 4px;
		font-family: 'DM Sans', system-ui, sans-serif;
		font-size: 12px;
		font-weight: 700;
		pointer-events: none;
		white-space: nowrap;
	}
	.wr {
		padding: 1px 5px;
		border-radius: 5px;
		background: rgba(0, 0, 0, 0.8);
	}
	.wr.good { color: #4ade80; }
	.wr.bad { color: #f87171; }
	.wr.neutral { color: #e5e7eb; }
	.wr.player { background: rgba(20, 30, 60, 0.9); }
</style>
