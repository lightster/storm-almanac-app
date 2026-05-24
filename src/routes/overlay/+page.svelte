<script>
	import { onMount, onDestroy } from 'svelte';
	import { getCurrentWindow } from '@tauri-apps/api/window';
	import { listen } from '@tauri-apps/api/event';
	import { invoke } from '@tauri-apps/api/core';
	import { page } from '$app/state';

	const BADGE_FONT_K = 0.16;
	const BADGE_FONT_FLOOR_PX = 10;

	let mode = $state('interactive');
	let counter = $state(0);
	/** @type {'blocking' | 'interactable'} */
	let blockerMode = $state('blocking');
	/** @type {{ hero: string, player_name: string, is_self: boolean, rect: {x:number,y:number,width:number,height:number}, circle: {x:number,y:number,width:number,height:number}, win_rates: {overall: number|null, player: number|null, player_games: number|null} }[]} */
	let draftHeroes = $state([]);
	/** @type {{dataUrl: string, scale: number} | null} */
	let testDraft = $state(null);
	let flashing = $state(false);
	/** @type {(() => void) | undefined} */
	let unlisten;
	/** @type {ReturnType<typeof setTimeout> | undefined} */
	let flashTimer;

	onMount(async () => {
		const m = page.url.searchParams.get('mode');
		mode = m === 'clickthrough' || m === 'blocker' || m === 'draft' || m === 'test-draft' ? m : 'interactive';

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
			// The overlay window persists once created, so it mounts only once.
			// Each draft pushes fresh data via this event.
			unlisten = await listen('draft://update', (event) => {
				const heroes = event?.payload?.heroes;
				if (Array.isArray(heroes)) {
					draftHeroes = heroes;
				}
			});
		}

		if (mode === 'test-draft') {
			try {
				const data = await invoke('test_mode_get_current');
				if (data && typeof data.dataUrl === 'string') {
					testDraft = {
						dataUrl: data.dataUrl,
						scale: typeof data.scale === 'number' ? data.scale : 1,
					};
				}
			} catch (e) {
				console.error('test_mode_get_current failed', e);
			}
			unlisten = await listen('test-draft://load', (event) => {
				const p = event?.payload;
				console.log(
					'test-draft://load received:',
					p ? `dataUrl ${p.dataUrl?.length ?? '?'} bytes, scale ${p.scale}` : 'invalid payload'
				);
				if (p && typeof p.dataUrl === 'string') {
					testDraft = {
						dataUrl: p.dataUrl,
						scale: typeof p.scale === 'number' ? p.scale : 1,
					};
				}
			});
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

	async function onTestDraftClick() {
		try {
			await invoke('test_mode_next');
		} catch (e) {
			console.error('test_mode_next failed', e);
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
		{@const fontSize = Math.max(BADGE_FONT_FLOOR_PX, h.circle.width * BADGE_FONT_K)}
		<div
			class="wr-badge overall {wrClass(h.win_rates.overall)}"
			style="left: {h.circle.x}px; top: {h.circle.y}px; font-size: {fontSize}px;"
		>
			{fmt(h.win_rates.overall)}
		</div>
		{#if h.win_rates.player !== null}
			<div
				class="wr-badge personal {wrClass(h.win_rates.player)}"
				style="left: {h.circle.x + h.circle.width}px; top: {h.circle.y}px; font-size: {fontSize}px;"
			>
				{h.is_self ? 'you' : h.player_name}&nbsp;{fmt(h.win_rates.player)}
			</div>
		{/if}
	{/each}
{:else if mode === 'test-draft'}
	{#if testDraft}
		<button
			class="test-draft-bg"
			onclick={onTestDraftClick}
			aria-label="advance to next test fixture"
		>
			<img
				src={testDraft.dataUrl}
				alt="ARAM draft test fixture"
				style="transform: scale({testDraft.scale}); transform-origin: top left;"
			/>
		</button>
	{/if}
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

	.wr-badge {
		position: absolute;
		font-family: 'DM Sans', system-ui, sans-serif;
		font-weight: 700;
		line-height: 1;
		padding: 0.22em 0.5em;
		border-radius: 0.43em;
		background: rgba(8, 10, 20, 0.82);
		border: 1px solid rgba(255, 255, 255, 0.16);
		box-shadow: 0 1px 4px rgba(0, 0, 0, 0.55);
		pointer-events: none;
		white-space: nowrap;
	}
	.wr-badge.overall {
		transform: translate(-100%, 0);
	}
	.wr-badge.personal {
		transform: none;
	}
	.wr-badge.good {
		color: #5ef08a;
	}
	.wr-badge.bad {
		color: #ff7676;
	}
	.wr-badge.neutral {
		color: #e8eaf0;
	}

	.test-draft-bg {
		position: fixed;
		inset: 0;
		width: 100vw;
		height: 100vh;
		margin: 0;
		padding: 0;
		border: 0;
		background: #000;
		cursor: pointer;
		overflow: hidden;
	}

	.test-draft-bg img {
		position: absolute;
		left: 0;
		top: 0;
		max-width: none;
		max-height: none;
		display: block;
	}
</style>
