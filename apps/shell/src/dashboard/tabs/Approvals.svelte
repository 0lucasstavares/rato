<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import HudPanel from "../../ui/hud/HudPanel.svelte";
  import { fmtAgo, poll, rpc } from "../../lib/rpc";
  import type { ApprovalDto } from "../../lib/types";

  let pending = $state<ApprovalDto[]>([]);
  let decided = $state<ApprovalDto[]>([]);
  let auditOpen = $state(false);
  let stop: (() => void) | null = null;

  // Per-card slug input values for R3 gate
  let slugInputs = $state<Map<string, string>>(new Map());
  // Per-card busy state
  let busy = $state<Set<string>>(new Set());

  async function load() {
    const all = await rpc<ApprovalDto[]>("approvals.pending");
    pending = all;
  }

  onMount(() => {
    stop = poll(load, 2000);
  });

  onDestroy(() => stop?.());

  function riskBorderColor(risk: number): string {
    if (risk <= 1) return "var(--hud-ok)";
    if (risk === 2) return "var(--hud-warn)";
    return "var(--hud-danger)";
  }

  function riskLabel(risk: number): string {
    if (risk === 0) return "R0";
    if (risk === 1) return "R1";
    if (risk === 2) return "R2";
    return "R3";
  }

  type InjectionInfo = {
    exact_bytes: string;
    include_enter: boolean;
    target: string | null;
    expected_command: string | null;
  };

  function injectionInfo(approval: ApprovalDto): InjectionInfo | null {
    const payload = approval.payload as Record<string, unknown> | null;
    if (!payload || typeof payload !== "object") return null;
    const exact = payload["exact_bytes"];
    if (typeof exact !== "string") return null;
    const includeEnter = Boolean(payload["include_enter"]);
    const target = typeof payload["target"] === "string" ? payload["target"] : null;
    const expected =
      typeof payload["expected_command"] === "string" ? payload["expected_command"] : null;

    return {
      exact_bytes: exact,
      include_enter: includeEnter,
      target,
      expected_command: expected,
    };
  }

  function slugFor(id: string): string {
    // Last 6 chars of the approval id — matches daemon logic
    return id.slice(-6);
  }

  function expiryCountdown(expiresAt: number): string {
    const ms = expiresAt - Date.now();
    if (ms <= 0) return "expired";
    const s = Math.floor(ms / 1000);
    if (s < 60) return `${s}s`;
    if (s < 3600) return `${Math.floor(s / 60)}m`;
    return `${Math.floor(s / 3600)}h`;
  }

  function diffstatText(approval: ApprovalDto): string {
    if (!approval.payload) return "";
    const p = approval.payload as Record<string, unknown>;
    if (typeof p["diffstat"] === "string") return p["diffstat"] as string;
    const ei = approval.expected_impact as Record<string, unknown> | null;
    if (ei && typeof ei["diffstat"] === "string") return ei["diffstat"] as string;
    return "";
  }

  async function decide(id: string, verdict: "approve" | "deny") {
    const nextBusy = new Set(busy);
    nextBusy.add(id);
    busy = nextBusy;
    try {
      const approval = pending.find((a) => a.id === id);
      const slug = approval && approval.risk >= 3 ? slugInputs.get(id) : undefined;
      await rpc("approvals.decide", { id, verdict, slug: slug ?? null });
      await load();
      // Clear slug input after action
      const nextInputs = new Map(slugInputs);
      nextInputs.delete(id);
      slugInputs = nextInputs;
    } finally {
      const nextBusy = new Set(busy);
      nextBusy.delete(id);
      busy = nextBusy;
    }
  }

  function setSlug(id: string, value: string) {
    const next = new Map(slugInputs);
    next.set(id, value);
    slugInputs = next;
  }

  function approveDisabled(approval: ApprovalDto): boolean {
    if (busy.has(approval.id)) return true;
    if (approval.risk >= 3) {
      const input = slugInputs.get(approval.id) ?? "";
      return input !== slugFor(approval.id);
    }
    return false;
  }
</script>

<div class="approvals-tab">
  {#if pending.length === 0}
    <div class="empty">no pending approvals — all clear</div>
  {:else}
    <div class="section-label">PENDING ({pending.length})</div>
    {#each pending as a (a.id)}
      {@const inj = injectionInfo(a)}
      <div class="card-wrap">
        <div
          class="approval-card hud-panel hud-grunge"
          style="--risk-color: {riskBorderColor(a.risk)};"
        >
          <div class="risk-stripe"></div>
          <div class="card-body">
            <div class="card-header">
              <span class="risk-badge" style="background: {riskBorderColor(a.risk)};">{riskLabel(a.risk)}</span>
              <span class="card-title">{a.title}</span>
              <span class="expiry mono" class:expiry-urgent={expiryCountdown(a.expires_at) === "expired"}>
                {expiryCountdown(a.expires_at)}
              </span>
            </div>

            <div class="card-reason">{a.reason}</div>
            <div class="voice-slug mono">voice slug: {a.spoken_slug}</div>

            {#if inj}
              <div class="inject-box">
                <div class="inject-meta">
                  {#if inj.target}
                    <span class="hud-chip inject-chip">target: {inj.target}</span>
                  {/if}
                  {#if inj.expected_command}
                    <span class="hud-chip inject-chip dim-chip">cmd: {inj.expected_command}</span>
                  {/if}
                  {#if inj.include_enter}
                    <span class="hud-chip inject-chip">Enter after paste</span>
                  {:else}
                    <span class="hud-chip inject-chip dim-chip">No Enter</span>
                  {/if}
                </div>
                <pre class="inject-bytes">{inj.exact_bytes}</pre>
                <div class="countdown-overlay" aria-hidden="true">{expiryCountdown(a.expires_at)}</div>
              </div>
            {/if}

            {#if diffstatText(a)}
              <pre class="diffstat-block">{diffstatText(a)}</pre>
            {/if}

            <div class="card-meta">
              <span class="mono">{a.kind}</span>
              <span class="dot-sep">·</span>
              <span class="mono">{a.agent_identity}</span>
              <span class="dot-sep">·</span>
              <span class="mono">{fmtAgo(a.created)} ago</span>
              {#if a.cwd}
                <span class="dot-sep">·</span>
                <span class="mono cwd" title={a.cwd}>{a.cwd.split("/").slice(-2).join("/")}</span>
              {/if}
            </div>

            {#if a.risk >= 3}
              <div class="slug-gate">
                <label class="slug-label mono" for="slug-{a.id}">
                  type <span class="slug-hint">{slugFor(a.id)}</span> to arm Approve
                </label>
                <input
                  id="slug-{a.id}"
                  class="slug-input mono"
                  type="text"
                  placeholder={slugFor(a.id)}
                  value={slugInputs.get(a.id) ?? ""}
                  oninput={(e) => setSlug(a.id, (e.target as HTMLInputElement).value)}
                />
              </div>
            {/if}

            <div class="card-actions">
              <button
                class="hud-btn approve-btn"
                disabled={approveDisabled(a)}
                onclick={() => decide(a.id, "approve")}
              >Approve</button>
              <button
                class="hud-btn deny-btn"
                disabled={busy.has(a.id)}
                onclick={() => decide(a.id, "deny")}
              >Deny</button>
            </div>
          </div>
        </div>
      </div>
    {/each}
  {/if}

  <!-- Audit list (decided/expired) — collapsed by default -->
  <div class="audit-section">
    <button
      class="audit-toggle hud-btn"
      onclick={() => (auditOpen = !auditOpen)}
    >{auditOpen ? "▲" : "▼"} AUDIT LOG</button>

    {#if auditOpen}
      {#if decided.length === 0}
        <div class="audit-empty mono">no audit records yet</div>
      {:else}
        {#each decided as a (a.id)}
          <div class="audit-row">
            <span class="risk-badge-sm" style="background: {riskBorderColor(a.risk)};">{riskLabel(a.risk)}</span>
            <span class="audit-status mono" class:status-approved={a.status === "approved"} class:status-denied={a.status === "denied"}>{a.status.toUpperCase()}</span>
            <span class="audit-title">{a.title}</span>
            <span class="mono dim">{a.decided_at ? fmtAgo(a.decided_at) + " ago" : fmtAgo(a.created) + " ago"}</span>
          </div>
        {/each}
      {/if}
    {/if}
  </div>
</div>

<style>
  .approvals-tab {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .empty {
    font-family: var(--hud-font-marker);
    font-size: 15px;
    color: var(--hud-ink-dim);
    text-align: center;
    padding: 40px 0;
  }

  .section-label {
    font-family: var(--hud-font-head);
    font-size: 12px;
    letter-spacing: 2px;
    color: var(--hud-ink-dim);
    padding: 0 2px;
  }

  .card-wrap {
    margin: 8px 0;
  }

  .approval-card {
    display: flex;
    flex-direction: row;
    overflow: hidden;
    position: relative;
  }

  /* Risk-striped left border */
  .risk-stripe {
    width: 6px;
    flex-shrink: 0;
    background: var(--risk-color, var(--hud-ok));
  }

  .card-body {
    flex: 1;
    padding: 10px 12px 10px;
    display: flex;
    flex-direction: column;
    gap: 7px;
  }

  .card-header {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .risk-badge {
    font-family: var(--hud-font-head);
    font-size: 10px;
    letter-spacing: 1px;
    color: var(--hud-panel);
    padding: 1px 6px;
    border-radius: 2px;
    flex-shrink: 0;
  }

  .card-title {
    font-family: var(--hud-font-head);
    font-size: 16px;
    letter-spacing: 0.4px;
    color: var(--hud-ink);
    flex: 1;
    line-height: 1.1;
  }

  .expiry {
    font-size: 11px;
    color: var(--hud-ink-dim);
    white-space: nowrap;
  }

  .expiry-urgent {
    color: var(--hud-danger);
  }

  .card-reason {
    font-family: var(--hud-font-body);
    font-size: 13px;
    color: var(--hud-ink);
    line-height: 1.4;
  }

  .voice-slug {
    display: inline-block;
    padding: 2px 6px;
    border: 1px solid color-mix(in srgb, var(--hud-ink) 35%, transparent);
    background: color-mix(in srgb, var(--hud-info) 8%, transparent);
    color: var(--hud-ink-dim);
  }

  .diffstat-block {
    font-family: var(--hud-font-data);
    font-size: 11px;
    color: var(--hud-ink);
    background: color-mix(in srgb, var(--hud-ink) 6%, var(--hud-panel));
    border: 1px solid color-mix(in srgb, var(--hud-ink) 20%, transparent);
    padding: 6px 8px;
    margin: 0;
    white-space: pre-wrap;
    word-break: break-all;
    max-height: 120px;
    overflow-y: auto;
  }

  .inject-box {
    position: relative;
    margin-top: 8px;
    border: 1px dashed color-mix(in srgb, var(--hud-ink) 45%, transparent);
    padding: 8px 10px;
    background: color-mix(in srgb, var(--hud-panel) 78%, transparent);
  }

  .inject-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 6px;
  }

  .inject-chip {
    font-size: 10px;
    letter-spacing: 0.5px;
    padding: 4px 6px;
    background: color-mix(in srgb, var(--hud-ink) 10%, transparent);
    border: 1px solid color-mix(in srgb, var(--hud-ink) 40%, transparent);
  }

  .dim-chip {
    opacity: 0.7;
  }

  .inject-bytes {
    font-family: var(--hud-font-data);
    font-size: 11px;
    color: var(--hud-ink);
    white-space: pre-wrap;
    word-break: break-word;
    margin: 0;
  }

  .countdown-overlay {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: flex-end;
    justify-content: flex-end;
    pointer-events: none;
    padding: 6px 8px;
    font-family: var(--hud-font-head);
    font-size: 18px;
    color: color-mix(in srgb, var(--hud-ink) 80%, transparent);
    text-shadow: 1px 1px 0 var(--hud-panel);
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

  .dot-sep {
    color: var(--hud-ink-dim);
  }

  .mono {
    font-family: var(--hud-font-data);
    font-size: 11px;
  }

  .cwd {
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* R3 slug gate */
  .slug-gate {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 0 2px;
    border-top: 1px dashed color-mix(in srgb, var(--hud-danger) 40%, transparent);
  }

  .slug-label {
    font-size: 11px;
    color: var(--hud-ink-dim);
    white-space: nowrap;
  }

  .slug-hint {
    color: var(--hud-danger);
    font-weight: bold;
  }

  .slug-input {
    font-family: var(--hud-font-data);
    font-size: 12px;
    width: 80px;
    background: var(--hud-panel);
    border: 2px solid var(--hud-ink);
    box-shadow: var(--hud-shadow-sm);
    padding: 3px 6px;
    color: var(--hud-ink);
    outline: none;
  }

  .slug-input:focus {
    border-color: var(--hud-danger);
  }

  .card-actions {
    display: flex;
    gap: 8px;
    padding-top: 4px;
  }

  .card-actions button.hud-btn {
    font-size: 11px;
    padding: 3px 12px;
  }

  .approve-btn:not(:disabled):hover {
    color: var(--hud-ok);
  }

  .deny-btn:not(:disabled):hover {
    color: var(--hud-danger);
  }

  button:disabled {
    opacity: 0.4;
    cursor: not-allowed;
    box-shadow: none;
  }

  /* Audit section */
  .audit-section {
    margin-top: 8px;
    border-top: 2px solid color-mix(in srgb, var(--hud-ink) 20%, transparent);
    padding-top: 10px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .audit-toggle {
    font-size: 10px;
    padding: 3px 10px;
    align-self: flex-start;
  }

  .audit-empty {
    color: var(--hud-ink-dim);
    font-size: 11px;
    padding: 6px 2px;
  }

  .audit-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 2px;
    border-bottom: 1px solid color-mix(in srgb, var(--hud-ink) 12%, transparent);
  }

  .risk-badge-sm {
    font-family: var(--hud-font-head);
    font-size: 9px;
    letter-spacing: 1px;
    color: var(--hud-panel);
    padding: 1px 4px;
    border-radius: 2px;
    flex-shrink: 0;
  }

  .audit-status {
    font-size: 10px;
    letter-spacing: 1px;
    color: var(--hud-ink-dim);
    flex-shrink: 0;
    width: 72px;
  }

  .status-approved {
    color: var(--hud-ok);
  }

  .status-denied {
    color: var(--hud-danger);
  }

  .audit-title {
    font-family: var(--hud-font-body);
    font-size: 12px;
    flex: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--hud-ink);
  }

  .dim {
    color: var(--hud-ink-dim);
  }
</style>
