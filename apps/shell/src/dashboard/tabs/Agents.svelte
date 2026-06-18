<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import HudPanel from "../../ui/hud/HudPanel.svelte";
  import StatusChip from "../../ui/hud/StatusChip.svelte";
  import { fmtAgo, optionalRpc, poll } from "../../lib/rpc";
  import type { AgentObservabilityDto, AgentWorkflowDto } from "../../lib/types";

  let observability = $state<AgentObservabilityDto>(fallbackObservability());
  let harnessUsage = $state<HarnessUsageRow[]>([]);
  let stop: (() => void) | null = null;
  const repoApi = "https://api.github.com/repos/0lucasstavares/rato";

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

  interface GitHubLabel {
    name: string;
  }

  interface GitHubIssue {
    labels: GitHubLabel[];
    pull_request?: unknown;
  }

  interface GitHubPullRequest {
    number: number;
  }

  interface GitHubRun {
    id: number;
    name: string;
    status: string;
    conclusion: string | null;
    event: string;
    created_at: string;
    updated_at: string;
    html_url: string;
  }

  interface GitHubJob {
    id: number;
    name: string;
    status: string;
    conclusion: string | null;
    started_at: string | null;
    completed_at: string | null;
    html_url: string;
  }

  interface HarnessUsageRow {
    workflow: string;
    harness: string;
    status: string;
    result: string;
    event: string;
    updated_ms: number;
    url: string;
  }

  function hasLabel(issue: GitHubIssue, name: string): boolean {
    return issue.labels.some((label) => label.name === name);
  }

  function workflowStatus(run: GitHubRun | undefined): string {
    if (!run) return "unknown";
    if (["queued", "in_progress", "requested", "waiting", "pending"].includes(run.status)) {
      return "running";
    }
    if (run.status === "completed" && run.conclusion === "success") return "passed";
    if (run.status === "completed" && run.conclusion === "cancelled") return "idle";
    if (run.status === "completed" && run.conclusion) return "failed";
    return "unknown";
  }

  function workflowResult(run: GitHubRun | undefined): string | null {
    if (!run) return "no public run found";
    if (run.status !== "completed") return `${run.status} via ${run.event}`;
    return `${run.conclusion ?? "completed"} via ${run.event}`;
  }

  function latestByWorkflow(runs: GitHubRun[], workflow: string): GitHubRun | undefined {
    return runs.find((run) => run.name === workflow);
  }

  function harnessFromJob(jobName: string): string {
    const lower = jobName.toLowerCase();
    if (lower.includes("codex")) return "codex";
    if (lower.includes("claude-code")) return "claude-code";
    if (lower.includes("anthropic")) return "claude-code";
    if (lower.includes("openai")) return "codex";
    return "agent";
  }

  function resultForJob(job: GitHubJob): string {
    if (job.status !== "completed") return job.status;
    if (job.conclusion === "failure" && harnessFromJob(job.name) === "codex") {
      return "failed / check quota";
    }
    if (job.conclusion === "failure" && harnessFromJob(job.name) === "claude-code") {
      return "failed / check quota";
    }
    return job.conclusion ?? "completed";
  }

  function fallbackUsageRowsFromRuns(runs: GitHubRun[]): HarnessUsageRow[] {
    return runs.slice(0, 30).map((run) => ({
      workflow: run.name,
      harness: "workflow",
      status: run.status,
      result: run.conclusion ?? run.status,
      event: run.event,
      updated_ms: Date.parse(run.updated_at || run.created_at),
      url: run.html_url,
    }));
  }

  async function usageRowsFromRuns(runs: GitHubRun[]): Promise<HarnessUsageRow[]> {
    const jobResponses = await Promise.all(
      runs.slice(0, 12).map(async (run) => {
        try {
          const response = await fetchJson<{ jobs: GitHubJob[] }>(`${repoApi}/actions/runs/${run.id}/jobs?per_page=20`);
          return { run, jobs: response.jobs };
        } catch {
          return { run, jobs: [] as GitHubJob[] };
        }
      }),
    );

    const rows = jobResponses.flatMap(({ run, jobs }) => {
      const agentJobs = jobs.filter((job) => job.name.toLowerCase().includes("codex") || job.name.toLowerCase().includes("claude-code"));
      if (agentJobs.length === 0) {
        return fallbackUsageRowsFromRuns([run]);
      }
      return agentJobs.map((job) => ({
        workflow: run.name,
        harness: harnessFromJob(job.name),
        status: job.status,
        result: resultForJob(job),
        event: run.event,
        updated_ms: Date.parse(job.completed_at || job.started_at || run.updated_at || run.created_at),
        url: job.html_url || run.html_url,
      }));
    });

    return rows.slice(0, 30);
  }

  function withGitHubRuns(base: AgentObservabilityDto, runs: GitHubRun[]): AgentWorkflowDto[] {
    return base.workflows.map((workflow) => {
      const run = latestByWorkflow(runs, workflow.workflow);
      return {
        ...workflow,
        status: workflowStatus(run),
        last_run_ms: run ? Date.parse(run.updated_at || run.created_at) : null,
        last_result: workflowResult(run),
      };
    });
  }

  async function fetchJson<T>(url: string): Promise<T> {
    const response = await fetch(url, {
      headers: {
        Accept: "application/vnd.github+json",
      },
    });
    if (!response.ok) {
      throw new Error(`GitHub API ${response.status} for ${url}`);
    }
    return (await response.json()) as T;
  }

  async function githubObservability(): Promise<AgentObservabilityDto> {
    const base = fallbackObservability();
    const [issues, pulls, runsResponse] = await Promise.all([
      fetchJson<GitHubIssue[]>(`${repoApi}/issues?state=open&per_page=100`),
      fetchJson<GitHubPullRequest[]>(`${repoApi}/pulls?state=open&per_page=50`),
      fetchJson<{ workflow_runs: GitHubRun[] }>(`${repoApi}/actions/runs?per_page=50`),
    ]);

    const realIssues = issues.filter((issue) => !issue.pull_request);
    const agentRuns = runsResponse.workflow_runs.filter((run) => run.name.startsWith("agent-"));
    harnessUsage = await usageRowsFromRuns(agentRuns);
    const newestAgentRun = agentRuns[0];
    const newestAgentRunMs = newestAgentRun ? Date.parse(newestAgentRun.created_at) : 0;
    const recentlyActive = newestAgentRunMs > Date.now() - 8 * 60 * 60 * 1000;

    return {
      autonomy: recentlyActive ? "on" : "unknown",
      source: "github-public",
      last_updated_ms: Date.now(),
      queue: {
        ready: realIssues.filter((issue) => hasLabel(issue, "ai:ready")).length,
        working: realIssues.filter((issue) => hasLabel(issue, "ai:working")).length,
        review: realIssues.filter((issue) => hasLabel(issue, "ai:review")).length,
        merge: realIssues.filter((issue) => hasLabel(issue, "ai:merge")).length,
        blocked: realIssues.filter((issue) => hasLabel(issue, "ai:blocked")).length,
        open_prs: pulls.length,
      },
      workflows: withGitHubRuns(base, agentRuns),
    };
  }

  async function load() {
    const fallback = fallbackObservability();
    const rpcData = await optionalRpc<AgentObservabilityDto>("agents.observability", null, fallback);
    if (rpcData.source !== "configured") {
      observability = rpcData;
    } else {
      observability = await githubObservability().catch(() => rpcData);
    }
  }

  onMount(() => {
    stop = poll(load, 30000);
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
    if (value === "on" || value.startsWith("on")) return "on";
    if (value === "off") return "off";
    return "warn";
  }

  function usageState(row: HarnessUsageRow): "on" | "warn" | "err" | "off" {
    if (row.status !== "completed") return "on";
    if (row.result === "success") return "on";
    if (row.result === "cancelled" || row.result === "skipped") return "warn";
    return "err";
  }

  function isQuotaRisk(row: HarnessUsageRow): boolean {
    return row.result.includes("quota") || row.result.includes("limit") || row.result === "failure";
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
    <HudPanel title="HARNESS USAGE">
      {#if harnessUsage.length === 0}
        <div class="empty">no public agent workflow usage loaded yet</div>
      {:else}
        <div class="usage-scroll">
          {#each harnessUsage as row (`${row.workflow}-${row.harness}-${row.updated_ms}-${row.url}`)}
            <div class="usage-row" class:quota-risk={isQuotaRisk(row)}>
              <div class="usage-main">
                <span class="mono workflow-name">{row.workflow}</span>
                <span class="harness-name">{row.harness}</span>
              </div>
              <StatusChip label={row.result.toUpperCase()} state={usageState(row)} />
              <span class="mono event-name">{row.event}</span>
              <a class="mono age-cell" href={row.url} target="_blank" rel="noreferrer">{fmtAgo(row.updated_ms)} ago</a>
            </div>
          {/each}
        </div>
      {/if}
    </HudPanel>

    <HudPanel title="LIVE SOURCE">
      <div class="bridge">
        <div class="bridge-title">GitHub observability</div>
        <code>{observability.source}</code>
        <p>
          The browser view reads public GitHub issues, pull requests, Actions runs, and matrix jobs directly.
          A future daemon RPC can replace this with authenticated repo variables and richer logs.
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

  .usage-scroll {
    display: flex;
    flex-direction: column;
    gap: 1px;
    max-height: 240px;
    overflow-y: auto;
    padding-right: 6px;
    scrollbar-color: var(--hud-ink) color-mix(in srgb, var(--hud-ink) 10%, transparent);
  }

  .usage-scroll::-webkit-scrollbar {
    width: 9px;
  }

  .usage-scroll::-webkit-scrollbar-track {
    background: color-mix(in srgb, var(--hud-ink) 8%, transparent);
  }

  .usage-scroll::-webkit-scrollbar-thumb {
    background: var(--hud-ink);
    border: 2px solid var(--hud-panel);
  }

  .usage-row {
    display: grid;
    grid-template-columns: minmax(150px, 1fr) 86px 82px 68px;
    gap: 8px;
    align-items: center;
    padding: 6px 0;
    border-bottom: 1px solid color-mix(in srgb, var(--hud-ink) 12%, transparent);
  }

  .usage-row.quota-risk {
    background: color-mix(in srgb, var(--hud-danger) 16%, transparent);
    border: 1px solid var(--hud-danger);
    padding-left: 6px;
    padding-right: 6px;
  }

  .usage-row.quota-risk .harness-name,
  .usage-row.quota-risk .workflow-name,
  .usage-row.quota-risk .event-name,
  .usage-row.quota-risk .age-cell {
    color: var(--hud-danger);
  }

  .usage-main {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .harness-name {
    font-family: var(--hud-font-body);
    font-size: 13px;
    color: var(--hud-ink);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .workflow-name,
  .event-name,
  .age-cell {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  a.age-cell {
    color: var(--hud-ink-dim);
    text-decoration: none;
  }

  a.age-cell:hover {
    color: var(--hud-ink);
    text-decoration: underline;
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

