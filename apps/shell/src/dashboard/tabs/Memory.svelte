<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import HudPanel from "../../ui/hud/HudPanel.svelte";
  import MeterBar from "../../ui/hud/MeterBar.svelte";
  import StatusChip from "../../ui/hud/StatusChip.svelte";
  import { fmtAgo, optionalRpc, poll, rpc } from "../../lib/rpc";
  import type { DisclosureDto, HitDto, MemoryDto, PinDto, Project } from "../../lib/types";

  type MemoryFilter = "active" | "archived" | "all";

  let memories = $state<MemoryDto[]>([]);
  let disclosures = $state<DisclosureDto[]>([]);
  let pins = $state<PinDto[]>([]);
  let projects = $state<Map<string, Project>>(new Map());
  let query = $state("");
  let filter = $state<MemoryFilter>("active");
  let hits = $state<HitDto[]>([]);
  let searchBusy = $state(false);
  let stop: (() => void) | null = null;

  async function load() {
    const includeArchived = filter !== "active";
    const rows = await optionalRpc<MemoryDto[]>("memory.list", { include_archived: includeArchived, limit: 80 }, []);
    memories = filter === "archived" ? rows.filter((memory) => memory.archived) : rows;
    disclosures = await optionalRpc<DisclosureDto[]>("disclosures.recent", { limit: 40 }, []);
    pins = await optionalRpc<PinDto[]>("pins.list", null, []);
    const projectList = await rpc<Project[]>("projects.list");
    projects = new Map(projectList.map((project) => [project.id, project]));
  }

  onMount(() => {
    stop = poll(load, 7000);
  });

  onDestroy(() => stop?.());

  async function search() {
    const q = query.trim();
    if (!q) {
      hits = [];
      return;
    }
    searchBusy = true;
    try {
      hits = await optionalRpc<HitDto[]>("memory.search", { query: q, n: 12 }, []);
    } finally {
      searchBusy = false;
    }
  }

  function projectName(projectId: string | null): string {
    if (!projectId) return "personal";
    return projects.get(projectId)?.name ?? projectId.slice(0, 8);
  }

  function sourceCount(memory: MemoryDto): number {
    return Array.isArray(memory.source_event_ids) ? memory.source_event_ids.length : 0;
  }

  function jsonCount(value: unknown): number {
    return Array.isArray(value) ? value.length : 0;
  }

  function confidenceText(value: number): string {
    return `${Math.round(value * 100)}%`;
  }

  function setFilter(next: MemoryFilter) {
    filter = next;
    void load();
  }

  let activeMemories = $derived(memories.filter((memory) => !memory.archived));
  let archivedMemories = $derived(memories.filter((memory) => memory.archived));
  let manualPins = $derived(pins.filter((pin) => pin.kind === "manual").length);
  let autoPins = $derived(pins.filter((pin) => pin.kind === "auto").length);
</script>

<div class="memory-tab">
  <div class="toolbar">
    <div class="segmented" aria-label="Memory filter">
      <button class:active={filter === "active"} onclick={() => setFilter("active")}>Active</button>
      <button class:active={filter === "archived"} onclick={() => setFilter("archived")}>Archived</button>
      <button class:active={filter === "all"} onclick={() => setFilter("all")}>All</button>
    </div>

    <form class="search" onsubmit={(event) => { event.preventDefault(); void search(); }}>
      <input bind:value={query} placeholder="memory search" />
      <button class="hud-btn" disabled={searchBusy}>{searchBusy ? "..." : "Search"}</button>
    </form>
  </div>

  <div class="summary-grid">
    <HudPanel title="Memory Index">
      <div class="score-line">
        <span>{activeMemories.length}</span>
        <small>active</small>
        <span>{archivedMemories.length}</span>
        <small>archived</small>
      </div>
    </HudPanel>

    <HudPanel title="Pinned Evidence">
      <div class="chips">
        <StatusChip label="MAN" state={manualPins > 0 ? "on" : "off"} />
        <StatusChip label="AUTO" state={autoPins > 0 ? "warn" : "off"} />
      </div>
      <div class="dim">{pins.length} total pins</div>
    </HudPanel>

    <HudPanel title="Disclosures">
      <div class="score-line">
        <span>{disclosures.length}</span>
        <small>recent</small>
      </div>
      <div class="dim">{disclosures[0] ? `last ${fmtAgo(disclosures[0].ts)} ago` : "none recorded"}</div>
    </HudPanel>
  </div>

  {#if hits.length > 0}
    <HudPanel title="Search Hits">
      <div class="hit-list">
        {#each hits as hit (hit.id)}
          <div class="hit-row">
            <StatusChip label={hit.kind === "memory" ? "MEM" : "OBS"} state={hit.kind === "memory" ? "on" : "warn"} />
            <span class="mono id">{hit.id}</span>
            <MeterBar label="score" value={hit.score} max={1} text={hit.score.toFixed(3)} color="var(--hud-info)" />
          </div>
        {/each}
      </div>
    </HudPanel>
  {/if}

  <div class="content-grid">
    <HudPanel title="Notes Browser">
      {#each memories as memory (memory.id)}
        <article class="memory-row" class:archived={memory.archived}>
          <div class="memory-head">
            <span class="memory-title">{memory.title}</span>
            <span class="mono">{confidenceText(memory.confidence)}</span>
          </div>
          <p>{memory.body}</p>
          <div class="meta-line">
            <span>{memory.type}</span>
            <span>{projectName(memory.project_id)}</span>
            <span>{sourceCount(memory)} sources</span>
            <span>updated {fmtAgo(memory.updated)} ago</span>
          </div>
        </article>
      {:else}
        <div class="empty">no memories for this filter</div>
      {/each}
    </HudPanel>

    <div class="side-stack">
      <HudPanel title="Pins Gallery">
        {#each pins.slice(0, 12) as pin (pin.id)}
          <div class="pin-row">
            <StatusChip label={pin.media} state={pin.kind === "auto" ? "warn" : "on"} />
            <div class="pin-copy">
              <span>{pin.reason}</span>
              <small>{pin.kind} · {fmtAgo(pin.created)} ago</small>
            </div>
          </div>
        {:else}
          <div class="empty">no pins yet</div>
        {/each}
      </HudPanel>

      <HudPanel title="Disclosure Ledger">
        {#each disclosures as disclosure (disclosure.id)}
          <div class="disclosure-row">
            <div class="disclosure-head">
              <span class="purpose">{disclosure.purpose}</span>
              <span class="mono">{fmtAgo(disclosure.ts)} ago</span>
            </div>
            <div class="meta-line">
              <span>{disclosure.model}</span>
              <span>{jsonCount(disclosure.memory_ids)} memories</span>
              <span>{jsonCount(disclosure.observation_ids)} observations</span>
            </div>
          </div>
        {:else}
          <div class="empty">no disclosure rows</div>
        {/each}
      </HudPanel>
    </div>
  </div>
</div>

<style>
  .memory-tab {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .toolbar {
    display: flex;
    gap: 12px;
    align-items: center;
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
  .search {
    display: flex;
    gap: 8px;
    align-items: center;
    flex: 1;
    min-width: 260px;
  }
  .search input {
    min-width: 0;
    flex: 1;
    height: 30px;
    border: 2px solid var(--hud-ink);
    background: var(--hud-panel);
    color: var(--hud-ink);
    box-shadow: var(--hud-shadow-sm);
    font-family: var(--hud-font-data);
    font-size: 12px;
    padding: 3px 8px;
  }
  .summary-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(180px, 1fr));
    gap: 12px;
  }
  .score-line {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }
  .score-line span {
    font-family: var(--hud-font-head);
    font-size: 30px;
    color: var(--hud-ink);
  }
  .score-line small,
  .dim {
    font-family: var(--hud-font-data);
    font-size: 11px;
    color: var(--hud-ink-dim);
  }
  .chips {
    display: flex;
    gap: 6px;
    margin-bottom: 8px;
  }
  .hit-list {
    display: grid;
    grid-template-columns: repeat(2, minmax(260px, 1fr));
    gap: 8px;
  }
  .hit-row {
    display: grid;
    grid-template-columns: auto minmax(90px, 1fr) minmax(130px, 0.8fr);
    gap: 8px;
    align-items: center;
  }
  .content-grid {
    display: grid;
    grid-template-columns: minmax(420px, 1.15fr) minmax(320px, 0.85fr);
    gap: 12px;
  }
  .side-stack {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .memory-row {
    padding: 9px 0;
    border-bottom: 1px solid color-mix(in srgb, var(--hud-ink) 16%, transparent);
  }
  .memory-row.archived {
    opacity: 0.65;
  }
  .memory-head,
  .disclosure-head {
    display: flex;
    gap: 8px;
    align-items: center;
    justify-content: space-between;
  }
  .memory-title,
  .purpose {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--hud-font-head);
    font-size: 13px;
    color: var(--hud-ink);
  }
  p {
    margin: 5px 0;
    color: var(--hud-ink);
    font-size: 13px;
    line-height: 1.35;
  }
  .meta-line {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 12px;
    color: var(--hud-ink-dim);
    font-family: var(--hud-font-data);
    font-size: 10px;
  }
  .pin-row {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    padding: 6px 0;
    border-bottom: 1px solid color-mix(in srgb, var(--hud-ink) 14%, transparent);
  }
  .pin-copy {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .pin-copy span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--hud-ink);
    font-size: 12px;
  }
  .pin-copy small {
    color: var(--hud-ink-dim);
    font-family: var(--hud-font-data);
    font-size: 10px;
  }
  .disclosure-row {
    padding: 7px 0;
    border-bottom: 1px solid color-mix(in srgb, var(--hud-ink) 14%, transparent);
  }
  .mono {
    font-family: var(--hud-font-data);
    font-size: 11px;
    color: var(--hud-ink-dim);
  }
  .id {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .empty {
    font-family: var(--hud-font-marker);
    font-size: 15px;
    color: var(--hud-ink-dim);
    text-align: center;
    padding: 26px 0;
  }
  @media (max-width: 920px) {
    .summary-grid,
    .hit-list,
    .content-grid {
      grid-template-columns: 1fr;
    }
  }
</style>
