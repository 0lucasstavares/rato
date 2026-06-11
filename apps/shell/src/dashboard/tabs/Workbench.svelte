<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import HudPanel from "../../ui/hud/HudPanel.svelte";
  import StatusChip from "../../ui/hud/StatusChip.svelte";
  import { fmtAgo, poll, rpc } from "../../lib/rpc";
  import type { AgentRunDto, ApprovalDto } from "../../lib/types";

  let runs = $state<AgentRunDto[]>([]);
  let expanded = $state<Set<string>>(new Set());
  let tailLines = $state<Map<string, string>>(new Map());
  let tailPolls = $state<Map<string, () => void>>(new Map());
  let mergingRunIds = $state<Set<string>>(new Set());
  let stop: (() => void) | null = null;

  async function loadRuns() {
    runs = await rpc<AgentRunDto[]>("workbench.runs", { n: 50 });
  }

  onMount(() => {
    stop = poll(loadRuns, 2000);
  });

  onDestroy(() => {
    stop?.();
    for (const stopTail of tailPolls.values()) {
      stopTail();
    }
  });

  function toggleExpand(id: string) {
    const next = new Set(expanded);
    if (next.has(id)) {
      next.delete(id);
      // Stop tail poll for this run
      const stopTail = tailPolls.get(id);
      if (stopTail) {
        stopTail();
        const nextPolls = new Map(tailPolls);
        nextPolls.delete(id);
        tailPolls = nextPolls;
      }
    } else {
      next.add(id);
      // Start tail poll
      const stopTail = poll(async () => {
        try {
          const result = await rpc<{ lines: string }>("workbench.tail", { run_id: id, lines: 50 });
          const nextLines = new Map(tailLines);
          nextLines.set(id, result.lines ?? "");
          tailLines = nextLines;
        } catch {
          // run may have no tmux_target — ignore
        }
      }, 2000);
      const nextPolls = new Map(tailPolls);
      nextPolls.set(id, stopTail);
      tailPolls = nextPolls;
    }
    expanded = next;
  }

  async function triggerMergeBack(runId: string) {
    const next = new Set(mergingRunIds);
    next.add(runId);
    mergingRunIds = next;
    try {
      await rpc<ApprovalDto>("workbench.merge_back", { run_id: runId });
      await loadRuns();
    } finally {
      const after = new Set(mergingRunIds);
      after.delete(runId);
      mergingRunIds = after;
    }
  }

  function statusState(status: string): "on" | "warn" | "err" | "off" {
    if (status === "running") return "on";
    if (status === "done") return "warn";
    if (status === "merged") return "on";
    if (status === "failed") return "err";
    return "off";
  }

  function diffstatSummary(run: AgentRunDto): string {
    if (!run.diffstat) return "—";
    const d = run.diffstat as Record<string, unknown>;
    if (typeof d === "object") {
      const files = d["files_changed"] ?? d["files"];
      const ins = d["insertions"] ?? d["added"];
      const del = d["deletions"] ?? d["removed"];
      if (files !== undefined) return `${files} files +${ins ?? 0} -${del ?? 0}`;
    }
    // Diffstat may be a plain string summary stored as a value
    if (typeof run.diffstat === "string") return run.diffstat as string;
    return "—";
  }
</script>

<div class="workbench-tab">
  {#if runs.length === 0}
    <div class="empty">no runs yet — start an agent task to see runs here</div>
  {:else}
    <div class="runs-table">
      <div class="table-header">
        <span class="col-adapter">ADAPTER</span>
        <span class="col-title">TITLE</span>
        <span class="col-status">STATUS</span>
        <span class="col-started">STARTED</span>
        <span class="col-diff">DIFF</span>
        <span class="col-toggle"></span>
      </div>

      {#each runs as run (run.id)}
        <div class="run-block">
          <div class="run-row" class:expanded={expanded.has(run.id)}>
            <span class="col-adapter mono">{run.adapter}</span>
            <span class="col-title run-title">{run.task_title}</span>
            <span class="col-status">
              <StatusChip label={run.status.toUpperCase()} state={statusState(run.status)} />
            </span>
            <span class="col-started mono">{fmtAgo(run.started)} ago</span>
            <span class="col-diff mono">{diffstatSummary(run)}</span>
            <span class="col-toggle">
              <button
                class="hud-btn expand-btn"
                onclick={() => toggleExpand(run.id)}
                title={expanded.has(run.id) ? "Collapse" : "Expand tail"}
              >{expanded.has(run.id) ? "▲" : "▼"}</button>

              {#if run.status === "done"}
                <button
                  class="hud-btn merge-btn"
                  disabled={mergingRunIds.has(run.id)}
                  onclick={() => triggerMergeBack(run.id)}
                  title="Create a merge-back approval for this run"
                >{mergingRunIds.has(run.id) ? "…" : "Merge back"}</button>
              {/if}
            </span>
          </div>

          {#if expanded.has(run.id)}
            <div class="tail-block">
              <pre class="tail-output">{tailLines.get(run.id) ?? "loading…"}</pre>
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .workbench-tab {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .empty {
    font-family: var(--hud-font-marker);
    font-size: 15px;
    color: var(--hud-ink-dim);
    text-align: center;
    padding: 40px 0;
  }

  .runs-table {
    display: flex;
    flex-direction: column;
    gap: 0;
    border: 2px solid var(--hud-ink);
    box-shadow: var(--hud-shadow);
    background: var(--hud-panel);
  }

  .table-header {
    display: grid;
    grid-template-columns: 90px 1fr 100px 72px 100px 120px;
    gap: 0 8px;
    padding: 6px 10px 5px;
    background: var(--hud-ink);
    color: var(--hud-panel);
    font-family: var(--hud-font-head);
    font-size: 10px;
    letter-spacing: 1px;
  }

  .run-block {
    border-top: 1px solid color-mix(in srgb, var(--hud-ink) 20%, transparent);
  }

  .run-row {
    display: grid;
    grid-template-columns: 90px 1fr 100px 72px 100px 120px;
    gap: 0 8px;
    padding: 7px 10px;
    align-items: center;
  }

  .run-row.expanded {
    background: color-mix(in srgb, var(--hud-accent) 8%, var(--hud-panel));
  }

  .run-title {
    font-family: var(--hud-font-body);
    font-size: 13px;
    color: var(--hud-ink);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .mono {
    font-family: var(--hud-font-data);
    font-size: 11px;
    color: var(--hud-ink-dim);
  }

  .col-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .expand-btn {
    font-size: 10px;
    padding: 2px 6px;
    line-height: 1;
  }

  .merge-btn {
    font-size: 10px;
    padding: 2px 6px;
    line-height: 1;
    white-space: nowrap;
  }

  .tail-block {
    padding: 0 10px 10px 10px;
    background: color-mix(in srgb, var(--hud-ink) 5%, var(--hud-panel));
    border-top: 1px dashed color-mix(in srgb, var(--hud-ink) 25%, transparent);
  }

  .tail-output {
    font-family: var(--hud-font-data);
    font-size: 11px;
    color: var(--hud-ink);
    line-height: 1.45;
    margin: 0;
    padding: 8px 0;
    white-space: pre-wrap;
    word-break: break-all;
    max-height: 260px;
    overflow-y: auto;
  }
</style>
