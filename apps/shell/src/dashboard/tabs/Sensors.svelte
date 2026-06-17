<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import HudPanel from "../../ui/hud/HudPanel.svelte";
  import MeterBar from "../../ui/hud/MeterBar.svelte";
  import StatusChip from "../../ui/hud/StatusChip.svelte";
  import { fmtAgo, optionalRpc, poll, rpc } from "../../lib/rpc";
  import type {
    ModeState,
    Observation,
    PinDto,
    RatEvent,
    RetentionStatusDto,
    RingMediaStatusDto,
    StatusResult,
    VoiceStatusDto,
  } from "../../lib/types";

  const AWAY_MS = 15 * 60 * 1000;

  let mode = $state<ModeState | null>(null);
  let events = $state<RatEvent[]>([]);
  let ocr = $state<Observation[]>([]);
  let pins = $state<PinDto[]>([]);
  let status = $state<StatusResult | null>(null);
  let voice = $state<VoiceStatusDto | null>(null);
  let retention = $state<RetentionStatusDto | null>(null);
  let ring = $state<RingMediaStatusDto[]>([]);
  let pinMinutes = $state(5);
  let pinBusy = $state(false);
  let pinMessage = $state<string | null>(null);
  let stop: (() => void) | null = null;

  async function load() {
    mode = await rpc<ModeState>("mode.get");
    status = await optionalRpc<StatusResult | null>("status", null, null);
    voice = await optionalRpc<VoiceStatusDto | null>("voice.status", null, null);
    events = await rpc<RatEvent[]>("events.recent", { limit: 300 });
    ocr = await rpc<Observation[]>("observations.recent", { limit: 20, kind: "ocr" });
    pins = await optionalRpc<PinDto[]>("pins.list", null, []);
    ring = await optionalRpc<RingMediaStatusDto[]>("ring.status", null, []);
    retention = await optionalRpc<RetentionStatusDto | null>("retention.status", null, null);
  }

  onMount(() => {
    stop = poll(load, 5000);
  });
  onDestroy(() => stop?.());

  async function pinRecentScreen() {
    pinBusy = true;
    pinMessage = null;
    try {
      const minutes = Math.max(1, Math.min(1440, Math.floor(Number(pinMinutes) || 1)));
      await rpc<PinDto>("pins.pin_recent", { media: "screen", minutes });
      pinMessage = `pinned last ${minutes} minute${minutes === 1 ? "" : "s"}`;
      await load();
    } catch (e) {
      pinMessage = e instanceof Error ? e.message : String(e);
    } finally {
      pinBusy = false;
    }
  }

  interface Row {
    name: string;
    led: "on" | "off" | "warn";
    note: string;
  }

  let board = $derived.by((): Row[] => {
    const bySource = new Map<string, RatEvent[]>();
    for (const e of events) {
      const list = bySource.get(e.source) ?? [];
      list.push(e);
      bySource.set(e.source, list);
    }
    const health = new Map((status?.sensors ?? []).map((sensor) => [sensor.name, sensor]));
    const row = (name: string, source: string, planned?: string): Row => {
      if (planned) return { name, led: "off", note: planned };
      const list = bySource.get(source);
      if (!list || list.length === 0) return { name, led: "warn", note: "no events yet" };
      return { name, led: "on", note: `${list.length} events · last ${fmtAgo(list[0].ts)} ago` };
    };
    const sensorRow = (name: string, sensorName: string, fallback: string): Row => {
      const sensor = health.get(sensorName);
      if (!sensor) return { name, led: "warn", note: fallback };
      if (sensor.state === "ok") return { name, led: "on", note: "backend healthy" };
      return { name, led: "warn", note: sensor.reason ?? "unavailable" };
    };
    const voiceBackendRow = (name: string, backendName: string): Row => {
      const backend = voice?.backends.find((b) => b.name === backendName);
      if (!backend) return { name, led: "warn", note: "not reported by daemon" };
      if (backend.state === "ok") return { name, led: "on", note: "backend healthy" };
      return { name, led: "warn", note: backend.reason ?? "unavailable" };
    };
    const retentionNote = retention
      ? `${retention.observations_deleted} obs · ${retention.pins_expired} pins · ${retention.api_calls_deleted} api calls · ${fmtAgo(retention.last_run_ms)} ago`
      : "nightly pruner has not run yet";
    const totalRingSegments = ring.reduce((sum, row) => sum + row.segment_count, 0);
    const ringNote = ring.length > 0
      ? `${totalRingSegments} segment(s) · ${ring.map((row) => `${row.media} ${row.segment_count}`).join(" · ")}`
      : "not reported by daemon";
    return [
      row("shell hooks", "shell"),
      row("processes", "proc"),
      row("git", "git"),
      row("clipboard", "clipboard"),
      row("idle/mode", "idle"),
      sensorRow("screen", "screen", "not reported by daemon"),
      sensorRow("ocr", "ocr", "not reported by daemon"),
      ocr.length > 0
        ? { name: "OCR observations", led: "on", note: `${ocr.length} recent · last ${fmtAgo(ocr[0].ts)} ago` }
        : { name: "OCR observations", led: "warn", note: "none stored yet" },
      pins.length > 0
        ? { name: "ring pins", led: "on", note: `${pins.length} pinned capture(s)` }
        : { name: "ring pins", led: "warn", note: "M5 encrypted ring ready · no pins yet" },
      { name: "ring occupancy", led: totalRingSegments > 0 ? "on" : "warn", note: ringNote },
      { name: "retention", led: retention ? "on" : "warn", note: retentionNote },
      voiceBackendRow("microphone", "mic"),
    ];
  });
</script>

<div class="col">
  <HudPanel title="Mode">
    {#if mode}
      <div class="mode-row">
        <StatusChip label={mode.mode} state={mode.mode === "away" ? "warn" : "on"} />
        {#if mode.idle_ms !== null}
          <MeterBar
            label="idle → away"
            value={mode.idle_ms}
            max={AWAY_MS}
            color={mode.idle_ms > AWAY_MS * 0.7 ? "var(--hud-warn)" : "var(--hud-accent)"}
            text="{Math.floor(mode.idle_ms / 1000)}s / {AWAY_MS / 60000}min"
          />
        {:else}
          <span class="dim">idle probe unavailable — activity fallback</span>
        {/if}
      </div>
    {/if}
  </HudPanel>

  <HudPanel title="Sensor Board">
    {#each board as s}
      <div class="sensor-row">
        <StatusChip label={s.name} state={s.led} />
        <span class="dim">{s.note}</span>
      </div>
    {/each}
    <div class="dim policy">all observation is read-only · no covert mode · M5 ring buffers are encrypted and pin-only</div>
  </HudPanel>

  <HudPanel title="Ring Capture">
    <div class="pin-row">
      <label>
        <span>minutes</span>
        <input type="number" min="1" max="1440" bind:value={pinMinutes} disabled={pinBusy} />
      </label>
      <button class="hud-btn" disabled={pinBusy} onclick={pinRecentScreen}>Pin Screen</button>
      {#if pinMessage}
        <span class="dim">{pinMessage}</span>
      {/if}
    </div>
  </HudPanel>
</div>

<style>
  .col {
    display: flex;
    flex-direction: column;
    gap: 12px;
    max-width: 720px;
  }
  .mode-row {
    display: flex;
    gap: 16px;
    align-items: center;
  }
  .sensor-row {
    display: flex;
    gap: 12px;
    align-items: center;
    padding: 4px 0;
  }
  .dim {
    color: var(--hud-ink-dim);
    font-size: 11px;
  }
  .policy {
    margin-top: 10px;
    border-top: 1px solid color-mix(in srgb, var(--hud-ink) 20%, transparent);
    padding-top: 6px;
  }
  .pin-row {
    display: flex;
    align-items: end;
    gap: 10px;
    flex-wrap: wrap;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-family: var(--hud-font-head);
    font-size: 10px;
    text-transform: uppercase;
    color: var(--hud-ink-dim);
  }
  input {
    width: 88px;
    height: 30px;
    border: 2px solid var(--hud-ink);
    background: var(--hud-panel);
    color: var(--hud-ink);
    box-shadow: var(--hud-shadow-sm);
    font-family: var(--hud-font-data);
    font-size: 12px;
    padding: 3px 8px;
  }
  button:disabled,
  input:disabled {
    opacity: 0.55;
    cursor: wait;
  }
</style>
