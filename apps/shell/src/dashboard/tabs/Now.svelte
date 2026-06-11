<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import HudPanel from "../../ui/hud/HudPanel.svelte";
  import MeterBar from "../../ui/hud/MeterBar.svelte";
  import { fmtAgo, fmtDuration, poll, rpc } from "../../lib/rpc";
  import type { Observation, Project, StatusResult, WorkSession } from "../../lib/types";

  let status = $state<StatusResult | null>(null);
  let sessions = $state<WorkSession[]>([]);
  let observations = $state<Observation[]>([]);
  let projects = $state<Map<string, Project>>(new Map());
  let stop: (() => void) | null = null;

  onMount(() => {
    stop = poll(async () => {
      status = await rpc<StatusResult>("status");
      sessions = await rpc<WorkSession[]>("sessions.recent", { limit: 10 });
      observations = await rpc<Observation[]>("observations.recent", { limit: 10 });
      const list = await rpc<Project[]>("projects.list");
      projects = new Map(list.map((p) => [p.id, p]));
    }, 5000);
  });
  onDestroy(() => stop?.());

  let openSessions = $derived(sessions.filter((s) => s.ended === null));
</script>

<div class="grid">
  <HudPanel title="Daemon">
    {#if status}
      <MeterBar label="uptime" value={status.uptime_ms} max={86_400_000} text={fmtDuration(status.uptime_ms)} />
      <MeterBar label="events" value={status.event_count % 1000} max={1000} text={String(status.event_count)} color="var(--hud-info)" />
      <div class="kv">ratd {status.version} · proto {status.proto_version}</div>
    {:else}
      <div class="kv err">daemon unreachable</div>
    {/if}
  </HudPanel>

  <HudPanel title="Active Work">
    {#if openSessions.length === 0}
      <div class="kv">no open work session — run something in a repo</div>
    {/if}
    {#each openSessions as s}
      <div class="session">
        <span class="proj">{projects.get(s.project_id)?.name ?? s.project_id.slice(0, 8)}</span>
        <span>{fmtDuration(s.last_activity - s.started)}</span>
        <span>{s.commands} cmds</span>
        <span class="dim">last {fmtAgo(s.last_activity)} ago</span>
      </div>
    {/each}
  </HudPanel>

  <HudPanel title="Mission Log">
    {#each observations as o}
      <div class="log-row">
        <span class="dim t">{fmtAgo(o.ts)}</span>
        <span class="kind">{o.kind}</span>
        <span class="content">{o.content}</span>
      </div>
    {:else}
      <div class="kv">no observations yet</div>
    {/each}
  </HudPanel>
</div>

<style>
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }
  .grid > :global(:last-child) {
    grid-column: 1 / -1;
  }
  .kv {
    font-size: 11px;
    color: var(--hud-text-dim);
    margin-top: 6px;
  }
  .kv.err {
    color: var(--hud-danger);
  }
  .session {
    display: flex;
    gap: 14px;
    align-items: baseline;
    padding: 4px 0;
  }
  .proj {
    color: var(--hud-accent);
    font-family: var(--hud-font-head);
    font-size: 11px;
    text-transform: uppercase;
  }
  .dim {
    color: var(--hud-text-dim);
    font-size: 11px;
  }
  .log-row {
    display: flex;
    gap: 10px;
    padding: 3px 0;
    border-bottom: 1px solid var(--hud-line-dark);
  }
  .log-row .t {
    width: 36px;
    text-align: right;
  }
  .log-row .kind {
    color: var(--hud-info);
    width: 130px;
    font-size: 11px;
  }
  .log-row .content {
    flex: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
