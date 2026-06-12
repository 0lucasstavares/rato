<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import HudPanel from "../../ui/hud/HudPanel.svelte";
  import StatusChip from "../../ui/hud/StatusChip.svelte";
  import { fmtAgo, fmtDuration, poll, rpc } from "../../lib/rpc";
  import type { PinDto } from "../../lib/types";

  let pins = $state<PinDto[]>([]);
  let media = $state("screen");
  let minutes = $state(5);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let stop: (() => void) | null = null;

  async function load() {
    pins = await rpc<PinDto[]>("pins.list");
  }

  onMount(() => {
    stop = poll(load, 5000);
  });

  onDestroy(() => stop?.());

  async function pinRecent() {
    busy = true;
    error = null;
    try {
      const clampedMinutes = Math.max(1, Math.min(1440, Math.floor(Number(minutes) || 1)));
      await rpc<PinDto>("pins.pin_recent", { media, minutes: clampedMinutes });
      await load();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  async function unpin(id: string) {
    busy = true;
    error = null;
    try {
      await rpc("pins.unpin", { id });
      await load();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  function expiry(pin: PinDto): string {
    if (pin.expires_at === null) return "manual";
    if (pin.expires_at <= Date.now()) return "expired";
    return `expires in ${fmtDuration(pin.expires_at - Date.now())}`;
  }

  function segmentCount(pin: PinDto): string {
    const n = pin.meta["segment_count"];
    return typeof n === "number" ? `${n} segment${n === 1 ? "" : "s"}` : "segments";
  }
</script>

<div class="pins-tab">
  <HudPanel title="Pin Recent">
    <div class="controls">
      <label>
        <span>media</span>
        <select bind:value={media} disabled={busy}>
          <option value="screen">screen</option>
          <option value="clipboard">clipboard</option>
          <option value="audio">audio</option>
        </select>
      </label>
      <label>
        <span>minutes</span>
        <input type="number" min="1" max="1440" bind:value={minutes} disabled={busy} />
      </label>
      <button class="hud-btn" disabled={busy} onclick={pinRecent}>Pin</button>
    </div>
    {#if error}
      <div class="error">{error}</div>
    {/if}
  </HudPanel>

  <HudPanel title="Pinned Ring">
    {#if pins.length === 0}
      <div class="empty">no pinned ring segments</div>
    {:else}
      <div class="pin-list">
        {#each pins as pin (pin.id)}
          <div class="pin-row">
            <div class="pin-main">
              <StatusChip label={pin.kind} state={pin.kind === "auto" ? "warn" : "on"} />
              <span class="media">{pin.media}</span>
              <span class="reason">{pin.reason}</span>
            </div>
            <div class="pin-meta">
              <span>{segmentCount(pin)}</span>
              <span>{fmtAgo(pin.created)} ago</span>
              <span>{expiry(pin)}</span>
              <span class="path" title={pin.path}>{pin.path}</span>
            </div>
            <button class="hud-btn unpin" disabled={busy} onclick={() => unpin(pin.id)}>Unpin</button>
          </div>
        {/each}
      </div>
    {/if}
  </HudPanel>
</div>

<style>
  .pins-tab {
    display: flex;
    flex-direction: column;
    gap: 12px;
    max-width: 860px;
  }

  .controls {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    align-items: end;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-family: var(--hud-font-head);
    font-size: 10px;
    letter-spacing: 1px;
    text-transform: uppercase;
    color: var(--hud-ink-dim);
  }

  select,
  input {
    min-width: 110px;
    height: 30px;
    border: 2px solid var(--hud-ink);
    background: var(--hud-panel);
    color: var(--hud-ink);
    box-shadow: var(--hud-shadow-sm);
    font-family: var(--hud-font-data);
    font-size: 12px;
    padding: 3px 8px;
  }

  input {
    width: 90px;
    min-width: 90px;
  }

  button:disabled,
  select:disabled,
  input:disabled {
    opacity: 0.55;
    cursor: wait;
  }

  .error {
    margin-top: 8px;
    color: var(--hud-danger);
    font-family: var(--hud-font-data);
    font-size: 11px;
  }

  .empty {
    font-family: var(--hud-font-marker);
    font-size: 15px;
    color: var(--hud-ink-dim);
    text-align: center;
    padding: 28px 0;
  }

  .pin-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .pin-row {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 5px 10px;
    padding: 8px 0;
    border-bottom: 1px solid color-mix(in srgb, var(--hud-ink) 18%, transparent);
  }

  .pin-main {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
    min-width: 0;
  }

  .media {
    font-family: var(--hud-font-head);
    font-size: 12px;
    color: var(--hud-info);
    text-transform: uppercase;
  }

  .reason {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--hud-ink);
  }

  .pin-meta {
    grid-column: 1 / -1;
    display: flex;
    flex-wrap: wrap;
    gap: 6px 12px;
    color: var(--hud-ink-dim);
    font-family: var(--hud-font-data);
    font-size: 10px;
  }

  .path {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .unpin {
    grid-column: 2;
    grid-row: 1;
    align-self: center;
    padding: 3px 8px;
    font-size: 10px;
  }
</style>
