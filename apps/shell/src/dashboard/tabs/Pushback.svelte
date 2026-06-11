<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import HudPanel from "../../ui/hud/HudPanel.svelte";
  import { fmtAgo, poll, rpc } from "../../lib/rpc";
  import type { PushbackDto } from "../../lib/types";

  let pushbacks = $state<PushbackDto[]>([]);
  let stop: (() => void) | null = null;

  async function load() {
    pushbacks = await rpc<PushbackDto[]>("pushbacks.recent", { n: 50 });
  }

  onMount(() => {
    stop = poll(load, 5000);
  });
  onDestroy(() => stop?.());

  async function sendFeedback(id: string, verdict: string) {
    // Optimistic update
    pushbacks = pushbacks.map((p) =>
      p.id === id ? { ...p, status: verdictToStatus(verdict) } : p
    );
    await rpc("pushbacks.feedback", { id, verdict });
    await load();
  }

  function verdictToStatus(verdict: string): string {
    if (verdict === "useful") return "accepted";
    if (verdict === "dismiss") return "dismissed";
    if (verdict === "snooze") return "snoozed";
    return verdict;
  }

  function severityState(severity: string): string {
    if (severity === "nudge") return "info";
    if (severity === "warn") return "warn";
    if (severity === "block-suggest") return "danger";
    return "info";
  }

  function severityColor(severity: string): string {
    if (severity === "nudge") return "var(--hud-info)";
    if (severity === "warn") return "var(--hud-warn)";
    if (severity === "block-suggest") return "var(--hud-danger)";
    return "var(--hud-info)";
  }

  function severityLabel(severity: string): string {
    if (severity === "nudge") return "NUDGE";
    if (severity === "warn") return "WARN";
    if (severity === "block-suggest") return "BLOCK";
    return severity.toUpperCase();
  }

  function canAct(p: PushbackDto): boolean {
    return p.status === "shown" || p.status === "queued";
  }
</script>

<div class="pushback-tab">
  {#if pushbacks.length === 0}
    <div class="empty">no pushback yet — keep skating</div>
  {:else}
    {#each pushbacks as p (p.id)}
      <div class="card-wrap">
        <HudPanel>
          <div class="card-inner">
            <div class="card-header">
              <span
                class="hud-chip sev-chip"
                style="--sev-color: {severityColor(p.severity)};"
              >
                <span class="dot"></span>{severityLabel(p.severity)}
              </span>
              <span class="hud-chip status-chip">{p.status.toUpperCase()}</span>
            </div>

            <div class="card-title">{p.title}</div>

            <div class="card-body">{p.message_en}</div>

            <div class="card-meta">
              <span class="trigger">{p.trigger}</span>
              <span class="dot-sep">·</span>
              <span>{p.evidence.length} evidence</span>
              <span class="dot-sep">·</span>
              <span>{fmtAgo(p.ts)} ago</span>
            </div>

            {#if canAct(p)}
              <div class="card-actions">
                <button class="hud-btn" onclick={() => sendFeedback(p.id, "useful")}>Useful</button>
                <button class="hud-btn" onclick={() => sendFeedback(p.id, "dismiss")}>Dismiss</button>
                <button class="hud-btn" onclick={() => sendFeedback(p.id, "snooze")}>Snooze</button>
              </div>
            {/if}
          </div>
        </HudPanel>
      </div>
    {/each}
  {/if}
</div>

<style>
  .pushback-tab {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .empty {
    font-family: var(--hud-font-marker);
    font-size: 15px;
    color: var(--hud-ink-dim);
    text-align: center;
    padding: 40px 0;
  }

  .card-wrap {
    /* Give HudPanel breathing room so tape strips don't clip */
    margin: 12px 0;
  }

  .card-inner {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .card-header {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .sev-chip {
    border-color: var(--sev-color, var(--hud-info));
  }

  .sev-chip .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    border: 1px solid var(--hud-ink);
    background: var(--sev-color, var(--hud-info));
    display: inline-block;
  }

  .status-chip {
    margin-left: auto;
    font-size: 9px;
    opacity: 0.75;
  }

  .card-title {
    font-family: var(--hud-font-head);
    font-size: 17px;
    letter-spacing: 0.5px;
    color: var(--hud-ink);
    line-height: 1.1;
  }

  .card-body {
    font-family: var(--hud-font-body);
    font-size: 12px;
    color: var(--hud-ink);
    line-height: 1.4;
  }

  .card-meta {
    font-family: var(--hud-font-marker);
    font-size: 10px;
    color: var(--hud-ink-dim);
    display: flex;
    gap: 4px;
    align-items: center;
    flex-wrap: wrap;
  }

  .trigger {
    color: var(--hud-accent);
  }

  .dot-sep {
    color: var(--hud-ink-dim);
  }

  .card-actions {
    display: flex;
    gap: 8px;
    padding-top: 4px;
  }

  .card-actions :global(.hud-btn),
  .card-actions button.hud-btn {
    font-size: 11px;
    padding: 3px 10px;
  }
</style>
