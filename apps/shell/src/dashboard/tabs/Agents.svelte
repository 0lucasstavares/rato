<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import HudPanel from "../../ui/hud/HudPanel.svelte";
  import StatusChip from "../../ui/hud/StatusChip.svelte";
  import { fmtAgo, optionalRpc, poll } from "../../lib/rpc";
  import type { AgentObservabilityDto, AgentRunDto, AgentWorkflowDto } from "../../lib/types";

  let observability = $state<AgentObservabilityDto>(fallbackObservability());
  let runs = $state<AgentRunDto[]>([]);
  let stop: (() => void) | null = null;

  const queueLabels: Array<{ key: keyof AgentObservabilityDto["queue"]; label: string }> = [
    { key: "ready", label: "READY" },
    { key: "working", label: "WORKING" },
    { key: "review", label: "REVIEW" },
    { key: "merge", label: "MERGE" },
    { key: "blocked", label: "BLOCKED" },
    { key: "open_prs", label: "OPEN PRS" },
  ];

  function fallbackObservability(): AgentObservabilityDto {
    return {
      autonomy: "unknown",
      source: "configured",
      last_updated_ms: Date.now(),
      queue: {
        ready: 0,
        working: 0,
        review: 0,
        merge: 0,
        blocked: 0,
        open_prs: 0,
      },
      workflows: [
        {
          role: "Manager",
          workflow: "agent-manager",
          status: "idle",
          trigger: "schedule + merger handoff",
          cadence: "every 2h while autonomy is on",
          last_run_ms: null,
          last_result: "waiting for live GitHub bridge",
          next_action: "triage ready issues and assign work",
          handoff: "agent-worker",
        },
        {
          role: "Worker",
          workflow: "agent-worker",
          status: "idle",
          trigger: "manager handoff + reviewer rework",
          cadence: "event-driven",
          last_run_ms: null,
          last_result: "waiting for live GitHub bridge",
          next_action: "open implementation PRs from assigned issues",
          handoff: "pull request",
        },
        {
          role: "Reviewer",
          workflow: "agent-reviewer",
          status: "idle",
          trigger: "pull request opened or updated",
          cadence: "event-driven",
          last_run_ms: null,
          last_result: "waiting for live GitHub bridge",
          next_action: "review PRs and request fixes when needed",
          handoff: "agent-worker or agent-merger",
        },
        {
          role: "Merger",
          workflow: "agent-merger",
          status: "idle",
          trigger: "CI + reviewer passed",
          cadence: "event-driven",
          last_run_ms: null,
          last_result: "waiting for live GitHub bridge",
          next_action: "merge clean PRs and kick the next manager pass",
          handoff: "agent-manager",
        },
      ],
    };
  }

  async function load() {
    const fallback = fallbackObservability();
    observability = await optionalRpc<AgentObservabilityDto>("agents.observability", null, fallback);
    runs = await optionalRpc<AgentRunDto[]>("workbench.runs", { n: 8 }, []);
  }

  onMount(() => {
    stop = poll(load, 5000);
  });

  onDestroy(() => {
    stop?.();
  });

  function chipState(status: string): "on" | "warn" | "err" | "off" {
    if (status === "running" || status === "passed") return "on";
    if (status === "failed" || status === "blocked") return "err";
    if (status === "unknown") return "warn";
    return "off";
  }

  function autonomyState(value: string): "on" | "warn" | "err" | "off" {
    if (value === "on") return "on";
    if (value === "off") return "off";
    return "warn";
  }

  function runState(run: AgentRunDto): "on" | "warn" | "err" | "off" {
    if (run.status === "running" || run.status === "merged") return "on";
    if (run.status === "failed") return "err";
    if (run.status === "done") return "warn";
    return "off";
  }

  function workflowAge(workflow: AgentWorkflowDto): string {
    if (!workflow.last_run_ms) return "no run";
    return `${fmtAgo(workflow.last_run_ms)} ago`;
  }
</script>

<div class="agents-tab">
  <section class="topline">
    <HudPanel title="AUTONOMY">
      <div class="status-row">
        <StatusChip label={observability.autonomy.toUpperCase()} state={autonomyState(observability.autonomy)} />
        <span class="source">{observability.source} source</span>
        <span class="age">updated {fmtAgo(observability.last_updated_ms)} ago</span>
      </div>
    </HudPanel>

    <HudPanel title="QUEUE">
      <div class="queue-grid">
        {#each queueLabels as item}
          <div class="metric">
            <span class="metric-value">{observability.queue[item.key]}</span>
            <span class="metric-label">{item.label}</span>
          </div>
        {/each}
      </div>
    </HudPanel>
  </section>

  <HudPanel title="AGENT LOOP">
    <div class="loop">
      {#each observability.workflows as workflow, i (workflow.workflow)}
        <article class="agent-card">
          <div class="agent-head">
            <div>
              <div class="role">{workflow.role}</div>
              <div class="workflow mono">{workflow.workflow}</div>
            </div>
            <StatusChip label={workflow.status.toUpperCase()} state={chipState(workflow.status)} />
          </div>

          <dl>
            <div>
              <dt>Trigger</dt>
              <dd>{workflow.trigger}</dd>
            </div>
            <div>
              <dt>Cadence</dt>
              <dd>{workflow.cadence}</dd>
            </div>
            <div>
              <dt>Last Run</dt>
              <dd>{workflowAge(workflow)}</dd>
            </div>
            <div>
              <dt>Next</dt>
              <dd>{workflow.next_action}</dd>
            </div>
          </dl>

          <div class="handoff">handoff -> {workflow.handoff}</div>
        </article>

        {#if i < observability.workflows.length - 1}
          <div class="connector" aria-hidden="true">-&gt;</div>
        {/if}
      {/each}
    </div>
  </HudPanel>

  <section class="bottom">
    <HudPanel title="RECENT LOCAL RUNS">
      {#if runs.length === 0}
        <div class="empty">no local workbench runs reported by this shell</div>
      {:else}
        <div class="runs">
          {#each runs as run (run.id)}
            <div class="run-row">
              <span class="mono adapter">{run.adapter}</span>
              <span class="title">{run.task_title}</span>
              <StatusChip label={run.status.toUpperCase()} state={runState(run)} />
              <span class="mono age-cell">{fmtAgo(run.started)} ago</span>
            </div>
          {/each}
        </div>
      {/if}
    </HudPanel>

    <HudPanel title="LIVE BRIDGE">
      <div class="bridge">
        <div class="bridge-title">Expected RPC</div>
        <code>agents.observability</code>
        <p>
          The tab is already wired for a daemon/GitHub bridge. Until that method exists, it renders
          the configured autonomous chain and local workbench runs.
        </p>
      </div>
    </HudPanel>
  </section>
</div>

<style>
  .agents-tab {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .topline,
  .bottom {
    display: grid;
    grid-template-columns: minmax(220px, 0.85fr) minmax(320px, 1.4fr);
    gap: 12px;
    align-items: stretch;
  }

  .status-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 12px;
    min-height: 52px;
  }

  .source,
  .age,
  .mono,
  code {
    font-family: var(--hud-font-data);
    font-size: 11px;
    color: var(--hud-ink-dim);
  }

  .queue-grid {
    display: grid;
    grid-template-columns: repeat(6, minmax(60px, 1fr));
    gap: 8px;
  }

  .metric {
    border: 1px solid color-mix(in srgb, var(--hud-ink) 25%, transparent);
    padding: 7px 8px;
    background: color-mix(in srgb, var(--hud-panel) 88%, white);
  }

  .metric-value {
    display: block;
    font-family: var(--hud-font-head);
    font-size: 24px;
    line-height: 1;
    color: var(--hud-ink);
  }

  .metric-label {
    display: block;
    margin-top: 3px;
    font-family: var(--hud-font-data);
    font-size: 10px;
    color: var(--hud-ink-dim);
  }

  .loop {
    display: grid;
    grid-template-columns: minmax(180px, 1fr) 24px minmax(180px, 1fr) 24px minmax(180px, 1fr) 24px minmax(180px, 1fr);
    gap: 8px;
    align-items: center;
  }

  .agent-card {
    min-height: 248px;
    border: 2px solid var(--hud-ink);
    background: color-mix(in srgb, var(--hud-panel) 92%, white);
    box-shadow: 3px 3px 0 color-mix(in srgb, var(--hud-ink) 20%, transparent);
    padding: 10px;
  }

  .agent-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 10px;
    margin-bottom: 10px;
  }

  .role {
    font-family: var(--hud-font-head);
    font-size: 20px;
    color: var(--hud-ink);
    line-height: 1;
  }

  .workflow {
    margin-top: 4px;
  }

  dl {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin: 0;
  }

  dt {
    font-family: var(--hud-font-head);
    font-size: 10px;
    color: var(--hud-ink);
    letter-spacing: 0.8px;
  }

  dd {
    margin: 2px 0 0;
    font-family: var(--hud-font-body);
    font-size: 12px;
    line-height: 1.25;
    color: var(--hud-ink-dim);
  }

  .handoff {
    margin-top: 10px;
    padding-top: 8px;
    border-top: 1px dashed color-mix(in srgb, var(--hud-ink) 35%, transparent);
    font-family: var(--hud-font-data);
    font-size: 11px;
    color: var(--hud-ink);
  }

  .connector {
    font-family: var(--hud-font-head);
    color: var(--hud-accent);
    text-align: center;
    font-size: 20px;
  }

  .runs {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .run-row {
    display: grid;
    grid-template-columns: 80px minmax(160px, 1fr) 82px 68px;
    gap: 8px;
    align-items: center;
    padding: 6px 0;
    border-bottom: 1px solid color-mix(in srgb, var(--hud-ink) 12%, transparent);
  }

  .title {
    font-family: var(--hud-font-body);
    font-size: 13px;
    color: var(--hud-ink);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .adapter,
  .age-cell {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .empty,
  .bridge p {
    font-family: var(--hud-font-marker);
    font-size: 14px;
    line-height: 1.35;
    color: var(--hud-ink-dim);
  }

  .bridge {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .bridge-title {
    font-family: var(--hud-font-head);
    font-size: 18px;
    color: var(--hud-ink);
  }

  code {
    width: fit-content;
    border: 1px solid color-mix(in srgb, var(--hud-ink) 28%, transparent);
    padding: 3px 6px;
    background: color-mix(in srgb, var(--hud-ink) 7%, transparent);
  }

  .bridge p {
    margin: 0;
  }

  @media (max-width: 1050px) {
    .topline,
    .bottom {
      grid-template-columns: 1fr;
    }

    .queue-grid {
      grid-template-columns: repeat(3, minmax(80px, 1fr));
    }

    .loop {
      grid-template-columns: 1fr;
    }

    .connector {
      display: none;
    }
  }
</style>

