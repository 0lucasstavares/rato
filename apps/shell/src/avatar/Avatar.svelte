<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { invoke } from "@tauri-apps/api/core";
  import StatusChip from "../ui/hud/StatusChip.svelte";
  import { poll, rpc } from "../lib/rpc";
  import type { ApprovalDto, ModeState, PushbackDto, StatusResult } from "../lib/types";
  import { mountRat, type Rat3D } from "./rat3d";

  let canvas: HTMLCanvasElement;
  let rat: Rat3D | null = null;

  let net = $state<"on" | "err">("err");
  let mode = $state<ModeState>({ mode: "active", since_ms: 0, idle_ms: null });
  let pendingApprovals = $state(0);
  let showQuick = $state(false);
  let stopPoll: (() => void) | null = null;

  // Pushback bubble state
  let bubble = $state<PushbackDto | null>(null);
  let bubbleTimer: ReturnType<typeof setTimeout> | null = null;

  // Module-level last-seen id (persists across re-renders but not page reloads)
  let lastSeenId = "";

  function clearBubbleTimer() {
    if (bubbleTimer !== null) {
      clearTimeout(bubbleTimer);
      bubbleTimer = null;
    }
  }

  function showBubble(p: PushbackDto) {
    bubble = p;
    lastSeenId = p.id;
    clearBubbleTimer();
    bubbleTimer = setTimeout(() => {
      bubble = null;
      bubbleTimer = null;
    }, 30_000);
  }

  function hideBubble() {
    bubble = null;
    clearBubbleTimer();
  }

  async function bubbleFeedback(id: string, verdict: string) {
    hideBubble();
    await rpc("pushbacks.feedback", { id, verdict });
  }

  onMount(() => {
    rat = mountRat(canvas);
    stopPoll = poll(async () => {
      try {
        await rpc<StatusResult>("status");
        net = "on";
        mode = await rpc<ModeState>("mode.get");
        rat?.setMode(mode.mode === "away" ? "away" : "active");

        // Pushback bubble check
        const pushbacks = await rpc<PushbackDto[]>("pushbacks.recent", { n: 1 });
        if (pushbacks.length > 0) {
          const newest = pushbacks[0];
          if (newest.status === "shown" && newest.id !== lastSeenId) {
            showBubble(newest);
          }
        }

        // Approvals pending count for APR chip
        try {
          const approvals = await rpc<ApprovalDto[]>("approvals.pending");
          pendingApprovals = approvals.length;
        } catch {
          pendingApprovals = 0;
        }
      } catch {
        net = "err";
      }
    }, 2000);
  });

  onDestroy(() => {
    stopPoll?.();
    rat?.dispose();
    clearBubbleTimer();
  });

  async function startDrag(e: MouseEvent) {
    if (e.buttons === 1) {
      await getCurrentWindow().startDragging();
    }
  }

  async function openDashboard() {
    showQuick = false;
    await invoke("open_dashboard");
  }

  function onBodyClick() {
    showQuick = !showQuick;
  }

  function onContextMenu(e: MouseEvent) {
    e.preventDefault();
    showQuick = !showQuick; // M2: right-click shares the quick panel; personality menu lands in M7
  }
</script>

<div class="avatar-root">
  <div
    class="grip"
    onmousedown={startDrag}
    role="toolbar"
    tabindex="-1"
    title="drag to move"
  >
    <StatusChip label="SCR" state="off" />
    <StatusChip label="MIC" state="off" />
    <StatusChip label="CLP" state={net === "on" ? "on" : "off"} />
    <StatusChip label="NET" state={net === "on" ? "on" : "err"} />
    {#if pendingApprovals > 0}
      <StatusChip label="APR" state="warn" />
    {/if}
  </div>

  <canvas
    bind:this={canvas}
    class="rat"
    width="150"
    height="180"
    onclick={onBodyClick}
    ondblclick={openDashboard}
    oncontextmenu={onContextMenu}
  ></canvas>

  {#if mode.mode === "away"}
    <div class="zzz">z Z z</div>
  {/if}

  {#if bubble !== null}
    <div class="pushback-bubble hud-panel hud-tape">
      <div class="bubble-title">{bubble.title}</div>
      <div class="bubble-msg">{bubble.message_en}</div>
      <div class="bubble-actions">
        <button class="hud-btn bubble-btn" onclick={() => bubble && bubbleFeedback(bubble.id, "useful")} title="Useful">✓</button>
        <button class="hud-btn bubble-btn" onclick={() => bubble && bubbleFeedback(bubble.id, "dismiss")} title="Dismiss">✕</button>
      </div>
    </div>
  {/if}

  {#if showQuick}
    <div class="quick hud-panel">
      <button class="hud-btn" onclick={openDashboard}>Dash</button>
      <button class="hud-btn" onclick={() => (showQuick = false)}>×</button>
      <div class="hint">{mode.mode}{mode.idle_ms !== null ? ` · idle ${Math.floor(mode.idle_ms / 1000)}s` : ""}</div>
    </div>
  {/if}
</div>

<style>
  .avatar-root {
    width: 200px;
    height: 240px;
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
  }
  /* tape band across the top: drag handle + sticker LEDs */
  .grip {
    display: flex;
    gap: 2px;
    justify-content: center;
    padding: 3px 0;
    width: 170px;
    background: var(--hud-tape);
    border: 1px solid rgba(30, 26, 21, 0.2);
    transform: rotate(-1.2deg);
    cursor: grab;
    z-index: 2;
  }
  .grip :global(.hud-chip) {
    font-size: 7px;
    padding: 1px 3px;
    border-width: 2px;
    box-shadow: none;
  }
  .rat {
    width: 180px;
    height: 216px;
    margin-top: auto;
    image-rendering: pixelated;
    cursor: pointer;
  }
  .zzz {
    position: absolute;
    top: 50px;
    right: 18px;
    font-family: var(--hud-font-marker);
    font-size: 15px;
    color: var(--hud-info);
    text-shadow: 1px 1px 0 #fff;
    animation: hud-blink-steps 2s steps(2) infinite;
    pointer-events: none;
  }
  .quick {
    position: absolute;
    bottom: 10px;
    left: 50%;
    rotate: none; /* neutralize .hud-panel's nth-child tilt — transform already rotates */
    transform: translateX(-50%) rotate(-1deg);
    display: flex;
    gap: 6px;
    align-items: center;
    padding: 7px;
    color: var(--hud-ink);
    z-index: 3;
  }
  .quick .hint {
    font-family: var(--hud-font-data);
    font-size: 11px;
    color: var(--hud-ink-dim);
    white-space: nowrap;
  }

  /* Pushback bubble: absolute paper card above the rat */
  .pushback-bubble {
    position: absolute;
    top: 24px; /* above canvas (canvas starts below grip ~30px) */
    left: 50%;
    transform: translateX(-50%) rotate(-1deg);
    width: 170px;
    z-index: 10;
    padding: 0;
    color: var(--hud-ink);
  }
  .bubble-title {
    font-family: var(--hud-font-body);
    font-size: 12px;
    padding: 6px 8px 2px;
    color: var(--hud-ink);
    line-height: 1.2;
    font-weight: normal;
  }
  .bubble-msg {
    font-family: var(--hud-font-body);
    font-size: 11px;
    line-height: 1.35;
    padding: 0 8px 4px;
    color: var(--hud-ink);
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .bubble-actions {
    display: flex;
    gap: 4px;
    padding: 4px 8px 8px;
  }
  .bubble-btn {
    font-size: 13px;
    padding: 2px 8px;
    line-height: 1;
  }
</style>
