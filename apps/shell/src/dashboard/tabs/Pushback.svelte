<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import HudPanel from "../../ui/hud/HudPanel.svelte";
  import { fmtAgo, poll, rpc } from "../../lib/rpc";
  import type { PushbackDto } from "../../lib/types";

  type StatusFilter = "open" | "decided" | "all";
  type SeverityFilter = "all" | "nudge" | "warn" | "block-suggest";

  let pushbacks = $state<PushbackDto[]>([]);
  let statusFilter = $state<StatusFilter>("open");
  let severityFilter = $state<SeverityFilter>("all");
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

  function statusMatches(p: PushbackDto): boolean {
    if (statusFilter === "all") return true;
    if (statusFilter === "open") return canAct(p);
    return !canAct(p);
  }

  function severityMatches(p: PushbackDto): boolean {
    return severityFilter === "all" || p.severity === severityFilter;
  }

  let filteredPushbacks = $derived(
    pushbacks.filter((p) => statusMatches(p) && severityMatches(p)),
  );
  let decidedCount = $derived(
    pushbacks.filter((p) => ["accepted", "dismissed", "snoozed"].includes(p.status)).length,
  );
  let acceptedCount = $derived(pushbacks.filter((p) => p.status === "accepted").length);
  let acceptanceRate = $derived(decidedCount === 0 ? 0 : Math.round((acceptedCount / decidedCount) * 100));
  let openCount = $derived(pushbacks.filter(canAct).length);
  let sparkPoints = $derived(pushbacks.slice(0, 24).reverse());
</script>

<div class="pushback-tab">
  <div class="toolbar">
    <div class="segmented" aria-label="Pushback status filter">
      <button class:active={statusFilter === "open"} onclick={() => (statusFilter = "open")}>Open</button>
      <button class:active={statusFilter === "decided"} onclick={() => (statusFilter = "decided")}>Decided</button>
      <button class:active={statusFilter === "all"} onclick={() => (statusFilter = "all")}>All</button>
    </div>
    <select bind:value={severityFilter} aria-label="Pushback severity filter">
      <option value="all">all severity</option>
      <option value="nudge">nudge</option>
      <option value="warn">warn</option>
      <option value="block-suggest">block</option>
    </select>
  </div>

  <div class="summary-strip">
    <div class="stat">
      <span>{openCount}</span>
      <small>open</small>
    </div>
    <div class="stat">
      <span>{acceptanceRate}%</span>
      <small>accepted</small>
    </div>
    <div class="sparkline" aria-label="Recent pushback outcomes">
      {#each sparkPoints as point (point.id)}
        <span
          class="spark"
          class:accepted={point.status === "accepted"}
          class:dismissed={point.status === "dismissed"}
          class:snoozed={point.status === "snoozed"}
          style="--spark-color: {severityColor(point.severity)};"
          title={`${point.status} · ${point.severity}`}
        ></span>
      {/each}
    </div>
  </div>

  {#if filteredPushbacks.length === 0}
    <div class="empty">no pushback yet — keep skating</div>
  {:else}
    {#each filteredPushbacks as p (p.id)}
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

            {#if p.evidence.length > 0}
              <div class="evidence-list">
                {#each p.evidence.slice(0, 3) as evidence}
                  <div class="evidence-row">
                    <span class="evidence-id">{evidence.observation_id.slice(0, 8)}</span>
                    <span>{evidence.quote}</span>
                  </div>
                {/each}
              </div>
            {/if}

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

  .toolbar,
  .summary-strip {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }

  .segmented {
    display: inline-flex;
    border: 2px solid var(--hud-ink);
    box-shadow: var(--hud-shadow-sm);
  }

  .segmented button {
    height: 30px;
    min-width: 74px;
    border: 0;
    border-right: 2px solid var(--hud-ink);
    background: var(--hud-panel);
    color: var(--hud-ink);
    font-family: var(--hud-font-head);
    font-size: 11px;
  }

  .segmented button:last-child {
    border-right: 0;
  }

  .segmented button.active {
    background: var(--hud-ink);
    color: var(--hud-panel);
  }

  select {
    height: 30px;
    border: 2px solid var(--hud-ink);
    background: var(--hud-panel);
    color: var(--hud-ink);
    box-shadow: var(--hud-shadow-sm);
    font-family: var(--hud-font-data);
    font-size: 12px;
    padding: 3px 8px;
  }

  .summary-strip {
    border: 2px solid color-mix(in srgb, var(--hud-ink) 36%, transparent);
    background: color-mix(in srgb, var(--hud-panel) 82%, transparent);
    padding: 7px 9px;
  }

  .stat {
    display: flex;
    align-items: baseline;
    gap: 5px;
  }

  .stat span {
    font-family: var(--hud-font-head);
    font-size: 22px;
    color: var(--hud-ink);
  }

  .stat small {
    font-family: var(--hud-font-data);
    font-size: 10px;
    color: var(--hud-ink-dim);
  }

  .sparkline {
    display: flex;
    align-items: end;
    gap: 3px;
    min-height: 22px;
    flex: 1;
    min-width: 180px;
  }

  .spark {
    display: block;
    width: 10px;
    height: 18px;
    background: var(--spark-color, var(--hud-info));
    border: 1px solid var(--hud-ink);
    opacity: 0.85;
  }

  .spark.accepted {
    height: 22px;
    background: var(--hud-accent);
  }

  .spark.dismissed {
    height: 9px;
    opacity: 0.5;
  }

  .spark.snoozed {
    height: 14px;
    background: var(--hud-warn);
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
    font-size: 14px;
    color: var(--hud-ink);
    line-height: 1.4;
  }

  .card-meta {
    font-family: var(--hud-font-data);
    font-size: 11px;
    color: var(--hud-ink-dim);
    display: flex;
    gap: 4px;
    align-items: center;
    flex-wrap: wrap;
  }

  .evidence-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 6px 0;
    border-top: 1px dashed color-mix(in srgb, var(--hud-ink) 22%, transparent);
    border-bottom: 1px dashed color-mix(in srgb, var(--hud-ink) 22%, transparent);
  }

  .evidence-row {
    display: grid;
    grid-template-columns: 76px 1fr;
    gap: 7px;
    align-items: baseline;
    font-family: var(--hud-font-data);
    font-size: 11px;
    color: var(--hud-ink-dim);
  }

  .evidence-row span:last-child {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .evidence-id {
    color: var(--hud-info);
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
