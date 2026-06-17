<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import HudPanel from "../../ui/hud/HudPanel.svelte";
  import MeterBar from "../../ui/hud/MeterBar.svelte";
  import StatusChip from "../../ui/hud/StatusChip.svelte";
  import { fmtAgo, fmtDuration, optionalRpc, poll, rpc } from "../../lib/rpc";
  import type {
    AgentRunDto,
    ApprovalDto,
    Observation,
    PinDto,
    Project,
    PushbackDto,
    RetentionStatusDto,
    RingMediaStatusDto,
    StatusResult,
    VoiceUtteranceDto,
    WorkSession,
  } from "../../lib/types";

  let status = $state<StatusResult | null>(null);
  let sessions = $state<WorkSession[]>([]);
  let observations = $state<Observation[]>([]);
  let projects = $state<Project[]>([]);
  let runs = $state<AgentRunDto[]>([]);
  let approvals = $state<ApprovalDto[]>([]);
  let pushbacks = $state<PushbackDto[]>([]);
  let pins = $state<PinDto[]>([]);
  let ring = $state<RingMediaStatusDto[]>([]);
  let retention = $state<RetentionStatusDto | null>(null);
  let utterances = $state<VoiceUtteranceDto[]>([]);
  let stop: (() => void) | null = null;

  async function load() {
    status = await rpc<StatusResult>("status");
    sessions = await rpc<WorkSession[]>("sessions.recent", { limit: 80 });
    observations = await rpc<Observation[]>("observations.recent", { limit: 300 });
    projects = await rpc<Project[]>("projects.list");
    runs = await optionalRpc<AgentRunDto[]>("workbench.runs", { n: 80 }, []);
    approvals = await optionalRpc<ApprovalDto[]>("approvals.pending", null, []);
    pushbacks = await optionalRpc<PushbackDto[]>("pushbacks.recent", { n: 80 }, []);
    pins = await optionalRpc<PinDto[]>("pins.list", null, []);
    ring = await optionalRpc<RingMediaStatusDto[]>("ring.status", null, []);
    retention = await optionalRpc<RetentionStatusDto | null>("retention.status", null, null);
    utterances = await optionalRpc<VoiceUtteranceDto[]>("voice.utterances", { limit: 40 }, []);
  }

  onMount(() => {
    stop = poll(load, 5000);
  });

  onDestroy(() => stop?.());

  const dayMs = 24 * 60 * 60 * 1000;
  let dayStart = $derived(Date.now() - dayMs);
  let activeSessions = $derived(sessions.filter((session) => session.ended === null).length);
  let daySessions = $derived(sessions.filter((session) => session.last_activity >= dayStart));
  let dayObservations = $derived(observations.filter((obs) => obs.ts >= dayStart));
  let dayPushbacks = $derived(pushbacks.filter((pb) => pb.ts >= dayStart));
  let acceptedPushbacks = $derived(pushbacks.filter((pb) => pb.status === "accepted").length);
  let decidedPushbacks = $derived(
    pushbacks.filter((pb) => ["accepted", "dismissed", "snoozed"].includes(pb.status)).length,
  );
  let acceptancePct = $derived(decidedPushbacks === 0 ? 0 : Math.round((acceptedPushbacks / decidedPushbacks) * 100));
  let runningRuns = $derived(runs.filter((run) => run.status === "running").length);
  let doneRuns = $derived(runs.filter((run) => run.status === "done" || run.status === "merged").length);
  let failedRuns = $derived(runs.filter((run) => run.status === "failed").length);
  let pendingR2 = $derived(approvals.filter((approval) => approval.risk === 2).length);
  let pendingR3 = $derived(approvals.filter((approval) => approval.risk >= 3).length);
  let autoPins = $derived(pins.filter((pin) => pin.kind === "auto").length);
  let manualPins = $derived(pins.filter((pin) => pin.kind === "manual").length);
  let ocrCount = $derived(dayObservations.filter((obs) => obs.kind === "ocr").length);
  let commandCount = $derived(dayObservations.filter((obs) => obs.kind === "shell_cmd").length);
  let agentOutputCount = $derived(dayObservations.filter((obs) => obs.kind === "agent_output").length);

  function latestActivity(): number | null {
    const values = [
      ...sessions.map((session) => session.last_activity),
      ...observations.map((obs) => obs.ts),
      ...pushbacks.map((pb) => pb.ts),
      ...runs.map((run) => run.ended ?? run.started),
      ...utterances.map((utterance) => utterance.ts),
    ];
    return values.length === 0 ? null : Math.max(...values);
  }

  function projectName(projectId: string): string {
    return projects.find((project) => project.id === projectId)?.name ?? projectId.slice(0, 8);
  }

  let topProjects = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const obs of dayObservations) {
      if (obs.project_id) counts.set(obs.project_id, (counts.get(obs.project_id) ?? 0) + 1);
    }
    return [...counts.entries()]
      .sort((a, b) => b[1] - a[1])
      .slice(0, 6);
  });

  let observationKinds = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const obs of dayObservations) {
      counts.set(obs.kind, (counts.get(obs.kind) ?? 0) + 1);
    }
    return [...counts.entries()]
      .sort((a, b) => b[1] - a[1])
      .slice(0, 8);
  });

  function ringLabel(item: RingMediaStatusDto): string {
    if (item.segment_count === 0) return "empty";
    const newest = item.newest_ms ? `${fmtAgo(item.newest_ms)} ago` : "unknown";
    return `${item.segment_count} seg · newest ${newest}`;
  }
</script>

<div class="metrics-tab">
  <div class="score-grid">
    <HudPanel title="Daemon">
      {#if status}
        <MeterBar label="uptime" value={status.uptime_ms % dayMs} max={dayMs} text={fmtDuration(status.uptime_ms)} />
        <MeterBar label="events" value={status.event_count % 1000} max={1000} text={String(status.event_count)} color="var(--hud-info)" />
        <div class="small-line">last activity {latestActivity() ? `${fmtAgo(latestActivity() as number)} ago` : "none"}</div>
      {:else}
        <div class="empty">daemon unreachable</div>
      {/if}
    </HudPanel>

    <HudPanel title="Today">
      <div class="big-stat">{dayObservations.length}</div>
      <div class="small-line">observations · {daySessions.length} sessions · {activeSessions} open</div>
      <div class="mini-meters">
        <MeterBar label="cmd" value={commandCount} max={Math.max(1, dayObservations.length)} text={String(commandCount)} />
        <MeterBar label="ocr" value={ocrCount} max={Math.max(1, dayObservations.length)} text={String(ocrCount)} color="var(--hud-warn)" />
        <MeterBar label="agent" value={agentOutputCount} max={Math.max(1, dayObservations.length)} text={String(agentOutputCount)} color="var(--hud-info)" />
      </div>
    </HudPanel>

    <HudPanel title="Critic">
      <div class="big-stat">{acceptancePct}%</div>
      <div class="small-line">{acceptedPushbacks} accepted · {dayPushbacks.length} in 24h</div>
      <MeterBar label="signal" value={acceptedPushbacks} max={Math.max(1, decidedPushbacks)} text={`${acceptedPushbacks}/${decidedPushbacks}`} color="var(--hud-accent)" />
    </HudPanel>

    <HudPanel title="Workbench">
      <div class="chips">
        <StatusChip label="RUN" state={runningRuns > 0 ? "warn" : "off"} />
        <StatusChip label="DONE" state={doneRuns > 0 ? "on" : "off"} />
        <StatusChip label="FAIL" state={failedRuns > 0 ? "err" : "off"} />
      </div>
      <div class="small-line">{runningRuns} running · {doneRuns} done/merged · {failedRuns} failed</div>
      <div class="small-line">{pendingR2} R2 pending · {pendingR3} R3 pending</div>
    </HudPanel>
  </div>

  <div class="detail-grid">
    <HudPanel title="Observation Mix">
      {#each observationKinds as [kind, count]}
        <div class="metric-row">
          <span class="key">{kind}</span>
          <MeterBar label="" value={count} max={Math.max(1, dayObservations.length)} text={String(count)} color="var(--hud-info)" />
        </div>
      {:else}
        <div class="empty">no observations in the last 24h</div>
      {/each}
    </HudPanel>

    <HudPanel title="Top Projects">
      {#each topProjects as [projectId, count]}
        <div class="metric-row">
          <span class="key">{projectName(projectId)}</span>
          <MeterBar label="" value={count} max={Math.max(1, dayObservations.length)} text={String(count)} color="var(--hud-accent)" />
        </div>
      {:else}
        <div class="empty">no project activity in the last 24h</div>
      {/each}
    </HudPanel>

    <HudPanel title="Ring + Retention">
      {#each ring as item}
        <div class="ring-row">
          <StatusChip label={item.media} state={item.segment_count > 0 ? "on" : "off"} />
          <span>{ringLabel(item)}</span>
        </div>
      {:else}
        <div class="empty">ring status unavailable</div>
      {/each}
      <div class="small-line retention">
        {#if retention}
          prune {fmtAgo(retention.last_run_ms)} ago · {retention.observations_deleted} obs · {retention.pins_expired} pins · {retention.api_calls_deleted} api
        {:else}
          no retention run recorded
        {/if}
      </div>
    </HudPanel>

    <HudPanel title="Pins + Voice">
      <div class="chips">
        <StatusChip label="AUTO" state={autoPins > 0 ? "warn" : "off"} />
        <StatusChip label="MAN" state={manualPins > 0 ? "on" : "off"} />
        <StatusChip label="MIC" state={utterances.length > 0 ? "on" : "off"} />
      </div>
      <div class="small-line">{autoPins} auto pins · {manualPins} manual pins</div>
      <div class="small-line">{utterances.length} recent utterances</div>
    </HudPanel>
  </div>
</div>

<style>
  .metrics-tab {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .score-grid,
  .detail-grid {
    display: grid;
    grid-template-columns: repeat(4, minmax(170px, 1fr));
    gap: 12px;
  }
  .detail-grid {
    grid-template-columns: repeat(2, minmax(280px, 1fr));
  }
  .big-stat {
    font-family: var(--hud-font-head);
    font-size: 34px;
    line-height: 1;
    color: var(--hud-ink);
    text-shadow: 2px 2px 0 var(--hud-accent);
  }
  .small-line,
  .empty {
    font-family: var(--hud-font-data);
    font-size: 11px;
    color: var(--hud-ink-dim);
    margin-top: 7px;
  }
  .empty {
    padding: 18px 0;
    text-align: center;
  }
  .mini-meters {
    margin-top: 8px;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 10px;
  }
  .metric-row {
    display: grid;
    grid-template-columns: minmax(90px, 150px) 1fr;
    gap: 10px;
    align-items: center;
    padding: 4px 0;
  }
  .key {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--hud-font-head);
    font-size: 11px;
    color: var(--hud-ink);
    text-transform: uppercase;
  }
  .ring-row {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 5px 0;
    border-bottom: 1px solid color-mix(in srgb, var(--hud-ink) 16%, transparent);
  }
  .ring-row span:last-child {
    color: var(--hud-ink-dim);
    font-family: var(--hud-font-data);
    font-size: 11px;
  }
  .retention {
    border-top: 1px dashed color-mix(in srgb, var(--hud-ink) 24%, transparent);
    padding-top: 8px;
  }
  @media (max-width: 900px) {
    .score-grid,
    .detail-grid {
      grid-template-columns: 1fr;
    }
  }
</style>
