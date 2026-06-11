<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { invoke } from "@tauri-apps/api/core";
  import StatusChip from "../ui/hud/StatusChip.svelte";
  import { poll, rpc } from "../lib/rpc";
  import type { ModeState, StatusResult } from "../lib/types";
  import { mountRat, type Rat3D } from "./rat3d";

  let canvas: HTMLCanvasElement;
  let rat: Rat3D | null = null;

  let net = $state<"on" | "err">("err");
  let mode = $state<ModeState>({ mode: "active", since_ms: 0, idle_ms: null });
  let showQuick = $state(false);
  let stopPoll: (() => void) | null = null;

  onMount(() => {
    rat = mountRat(canvas);
    stopPoll = poll(async () => {
      try {
        await rpc<StatusResult>("status");
        net = "on";
        mode = await rpc<ModeState>("mode.get");
        rat?.setMode(mode.mode === "away" ? "away" : "active");
      } catch {
        net = "err";
      }
    }, 2000);
  });

  onDestroy(() => {
    stopPoll?.();
    rat?.dispose();
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
  <!-- grip bar: drag handle -->
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
  </div>

  <!-- the rat -->
  <canvas
    bind:this={canvas}
    class="rat"
    width="240"
    height="240"
    onclick={onBodyClick}
    ondblclick={openDashboard}
    oncontextmenu={onContextMenu}
  ></canvas>

  {#if mode.mode === "away"}
    <div class="zzz">z Z z</div>
  {/if}

  {#if showQuick}
    <div class="quick hud-panel">
      <button class="hud-btn" onclick={openDashboard}>Dashboard</button>
      <button class="hud-btn" onclick={() => (showQuick = false)}>Close</button>
      <div class="hint">{mode.mode}{mode.idle_ms !== null ? ` · idle ${Math.floor(mode.idle_ms / 1000)}s` : ""}</div>
    </div>
  {/if}
</div>

<style>
  .avatar-root {
    width: 320px;
    height: 360px;
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
  }
  .grip {
    display: flex;
    gap: 3px;
    padding: 4px 6px;
    background: color-mix(in srgb, var(--hud-bg) 85%, transparent);
    border: 1px solid var(--hud-ink);
    cursor: grab;
    margin-top: 4px;
  }
  .rat {
    width: 320px;
    height: 320px;
    image-rendering: pixelated;
    cursor: pointer;
  }
  .zzz {
    position: absolute;
    top: 80px;
    right: 60px;
    font-family: var(--hud-font-head);
    color: var(--hud-info);
    animation: hud-blink-steps 2s steps(2) infinite;
    pointer-events: none;
  }
  .quick {
    position: absolute;
    bottom: 12px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    gap: 6px;
    align-items: center;
    padding: 8px;
    color: var(--hud-ink);
  }
  .quick .hint {
    font-size: 10px;
    color: var(--hud-ink-dim);
    white-space: nowrap;
  }
</style>
