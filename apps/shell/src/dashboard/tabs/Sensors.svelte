<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import HudPanel from "../../ui/hud/HudPanel.svelte";
  import MeterBar from "../../ui/hud/MeterBar.svelte";
  import StatusChip from "../../ui/hud/StatusChip.svelte";
  import { fmtAgo, poll, rpc } from "../../lib/rpc";
  import type { ModeState, RatEvent } from "../../lib/types";

  const AWAY_MS = 15 * 60 * 1000;

  let mode = $state<ModeState | null>(null);
  let events = $state<RatEvent[]>([]);
  let stop: (() => void) | null = null;

  onMount(() => {
    stop = poll(async () => {
      mode = await rpc<ModeState>("mode.get");
      events = await rpc<RatEvent[]>("events.recent", { limit: 300 });
    }, 5000);
  });
  onDestroy(() => stop?.());

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
    const row = (name: string, source: string, planned?: string): Row => {
      if (planned) return { name, led: "off", note: planned };
      const list = bySource.get(source);
      if (!list || list.length === 0) return { name, led: "warn", note: "no events yet" };
      return { name, led: "on", note: `${list.length} events · last ${fmtAgo(list[0].ts)} ago` };
    };
    return [
      row("shell hooks", "shell"),
      row("processes", "proc"),
      row("git", "git"),
      row("clipboard", "clipboard"),
      row("idle/mode", "idle"),
      row("screen", "", "arrives in M5"),
      row("microphone", "", "arrives in M6"),
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
    <div class="dim policy">all observation is read-only · no covert mode · raw buffers arrive in M5</div>
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
    color: var(--hud-text-dim);
    font-size: 11px;
  }
  .policy {
    margin-top: 10px;
    border-top: 1px solid var(--hud-line-dark);
    padding-top: 6px;
  }
</style>
