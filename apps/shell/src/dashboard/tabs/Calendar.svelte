<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import HudPanel from "../../ui/hud/HudPanel.svelte";
  import StatusChip from "../../ui/hud/StatusChip.svelte";
  import { fmtAgo, fmtDuration, poll, rpc } from "../../lib/rpc";
  import type { AgentRunDto, ApprovalDto, Observation, PinDto, Project, WorkSession } from "../../lib/types";

  type Zoom = "day" | "week";

  interface Marker {
    ts: number;
    glyph: string;
    label: string;
    tone: "on" | "warn" | "err" | "off";
  }

  let zoom = $state<Zoom>("day");
  let sessions = $state<WorkSession[]>([]);
  let observations = $state<Observation[]>([]);
  let runs = $state<AgentRunDto[]>([]);
  let approvals = $state<ApprovalDto[]>([]);
  let pins = $state<PinDto[]>([]);
  let projects = $state<Map<string, Project>>(new Map());
  let selected = $state<string | null>(null);
  let stop: (() => void) | null = null;

  async function load() {
    const limit = zoom === "day" ? 80 : 240;
    sessions = await rpc<WorkSession[]>("sessions.recent", { limit });
    observations = await rpc<Observation[]>("observations.recent", { limit: 500 });
    runs = await rpc<AgentRunDto[]>("workbench.runs", { n: 100 });
    approvals = await rpc<ApprovalDto[]>("approvals.pending");
    pins = await rpc<PinDto[]>("pins.list");
    const projectList = await rpc<Project[]>("projects.list");
    projects = new Map(projectList.map((project) => [project.id, project]));
  }

  onMount(() => {
    stop = poll(load, 5000);
  });
  onDestroy(() => stop?.());

  let windowStart = $derived(Date.now() - (zoom === "day" ? 24 : 7 * 24) * 60 * 60 * 1000);

  function projectName(projectId: string): string {
    return projects.get(projectId)?.name ?? projectId.slice(0, 8);
  }

  function within(ts: number): boolean {
    return ts >= windowStart;
  }

  function sessionMarkers(session: WorkSession): Marker[] {
    const ended = session.ended ?? Date.now();
    const projectObs = observations.filter(
      (obs) => obs.project_id === session.project_id && obs.ts >= session.started && obs.ts <= ended,
    );
    const projectRuns = runs.filter(
      (run) => run.project_id === session.project_id && run.started >= session.started && run.started <= ended,
    );
    const markers: Marker[] = [];

    const commandCount = projectObs.filter((obs) => obs.kind === "shell_cmd").length;
    if (commandCount > 0) markers.push({ ts: session.started, glyph: "CMD", label: `${commandCount} command${commandCount === 1 ? "" : "s"}`, tone: "on" });

    const ocrCount = projectObs.filter((obs) => obs.kind === "ocr").length;
    if (ocrCount > 0) markers.push({ ts: session.started, glyph: "OCR", label: `${ocrCount} screen note${ocrCount === 1 ? "" : "s"}`, tone: "warn" });

    for (const run of projectRuns.slice(0, 3)) {
      markers.push({ ts: run.started, glyph: "AGT", label: `${run.adapter} ${run.status}`, tone: run.status === "failed" ? "err" : "warn" });
    }

    return markers.sort((a, b) => a.ts - b.ts);
  }

  let visibleSessions = $derived(
    sessions
      .filter((session) => within(session.last_activity))
      .sort((a, b) => b.last_activity - a.last_activity),
  );

  let looseMarkers = $derived.by((): Marker[] => {
    const markers: Marker[] = [];
    for (const pin of pins) {
      if (within(pin.created)) markers.push({ ts: pin.created, glyph: "PIN", label: `${pin.media} ${pin.kind}`, tone: pin.kind === "auto" ? "warn" : "on" });
    }
    for (const approval of approvals) {
      if (within(approval.created)) markers.push({ ts: approval.created, glyph: `R${approval.risk}`, label: approval.title, tone: approval.risk >= 3 ? "err" : "warn" });
    }
    return markers.sort((a, b) => b.ts - a.ts).slice(0, 16);
  });

  let selectedSession = $derived(visibleSessions.find((session) => session.id === selected) ?? visibleSessions[0] ?? null);
</script>

<div class="calendar-tab">
  <div class="toolbar">
    <div class="segmented" aria-label="Calendar zoom">
      <button class:active={zoom === "day"} onclick={() => (zoom = "day")}>Day</button>
      <button class:active={zoom === "week"} onclick={() => (zoom = "week")}>Week</button>
    </div>
    <span class="dim">{visibleSessions.length} session{visibleSessions.length === 1 ? "" : "s"} · {looseMarkers.length} loose marker{looseMarkers.length === 1 ? "" : "s"}</span>
  </div>

  <div class="calendar-grid">
    <HudPanel title="Timeline">
      {#if visibleSessions.length === 0}
        <div class="empty">no sessions in this window</div>
      {:else}
        <div class="session-list">
          {#each visibleSessions as session (session.id)}
            <button
              class="session-row"
              class:selected={selectedSession?.id === session.id}
              onclick={() => (selected = session.id)}
            >
              <span class="time">{fmtAgo(session.last_activity)} ago</span>
              <span class="project">{projectName(session.project_id)}</span>
              <span class="duration">{fmtDuration((session.ended ?? Date.now()) - session.started)}</span>
              <span class="commands">{session.commands} cmd</span>
              <span class="markers">
                {#each sessionMarkers(session) as marker}
                  <StatusChip label={marker.glyph} state={marker.tone} />
                {/each}
              </span>
            </button>
          {/each}
        </div>
      {/if}
    </HudPanel>

    <HudPanel title="Session Detail">
      {#if selectedSession}
        <div class="detail-head">
          <span class="project big">{projectName(selectedSession.project_id)}</span>
          <span class="dim">{new Date(selectedSession.started).toLocaleString()}</span>
        </div>
        <div class="detail-kv">
          <span>{fmtDuration((selectedSession.ended ?? Date.now()) - selectedSession.started)}</span>
          <span>{selectedSession.commands} commands</span>
          <span>{selectedSession.ended ? "closed" : "open"}</span>
        </div>
        <div class="marker-list">
          {#each sessionMarkers(selectedSession) as marker}
            <div class="marker-row">
              <StatusChip label={marker.glyph} state={marker.tone} />
              <span>{marker.label}</span>
              <span class="dim">{fmtAgo(marker.ts)} ago</span>
            </div>
          {:else}
            <div class="dim">no markers attached to this session yet</div>
          {/each}
        </div>
      {:else}
        <div class="empty">select a session</div>
      {/if}
    </HudPanel>

    <HudPanel title="Pins + Approvals">
      {#each looseMarkers as marker}
        <div class="marker-row">
          <StatusChip label={marker.glyph} state={marker.tone} />
          <span>{marker.label}</span>
          <span class="dim">{fmtAgo(marker.ts)} ago</span>
        </div>
      {:else}
        <div class="empty">no pins or pending approvals in this window</div>
      {/each}
    </HudPanel>
  </div>
</div>

<style>
  .calendar-tab {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .segmented {
    display: inline-flex;
    border: 2px solid var(--hud-ink);
    box-shadow: var(--hud-shadow-sm);
  }
  .segmented button {
    height: 30px;
    min-width: 64px;
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
  .calendar-grid {
    display: grid;
    grid-template-columns: minmax(420px, 1.4fr) minmax(280px, 0.9fr);
    gap: 12px;
  }
  .calendar-grid > :global(:last-child) {
    grid-column: 1 / -1;
  }
  .session-list,
  .marker-list {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .session-row {
    display: grid;
    grid-template-columns: 72px minmax(110px, 1fr) 72px 64px minmax(120px, 0.9fr);
    gap: 8px;
    align-items: center;
    width: 100%;
    min-height: 34px;
    border: 1px solid color-mix(in srgb, var(--hud-ink) 35%, transparent);
    background: transparent;
    color: var(--hud-ink);
    text-align: left;
    padding: 5px 7px;
  }
  .session-row.selected {
    background: color-mix(in srgb, var(--hud-accent) 12%, var(--hud-panel));
    border-color: var(--hud-ink);
  }
  .time,
  .duration,
  .commands,
  .dim {
    color: var(--hud-ink-dim);
    font-family: var(--hud-font-data);
    font-size: 11px;
  }
  .project {
    color: var(--hud-accent);
    font-family: var(--hud-font-head);
    font-size: 12px;
    text-transform: uppercase;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .project.big {
    font-size: 16px;
  }
  .markers {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }
  .detail-head,
  .detail-kv,
  .marker-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .detail-head {
    justify-content: space-between;
  }
  .detail-kv {
    margin: 8px 0 12px;
    color: var(--hud-ink-dim);
    font-size: 11px;
  }
  .marker-row {
    padding: 4px 0;
    border-bottom: 1px solid color-mix(in srgb, var(--hud-ink) 16%, transparent);
  }
  .marker-row span:nth-child(2) {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .empty {
    font-family: var(--hud-font-marker);
    font-size: 15px;
    color: var(--hud-ink-dim);
    text-align: center;
    padding: 30px 0;
  }
  @media (max-width: 760px) {
    .calendar-grid {
      grid-template-columns: 1fr;
    }
    .calendar-grid > :global(:last-child) {
      grid-column: auto;
    }
    .session-row {
      grid-template-columns: 64px minmax(90px, 1fr) 58px;
    }
    .commands,
    .markers {
      display: none;
    }
  }
</style>
