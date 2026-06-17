<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import HudPanel from "../../ui/hud/HudPanel.svelte";
  import { optionalRpc, poll } from "../../lib/rpc";
  import type {
    DotfileEditDto,
    StatusResult,
    VoiceStatusDto,
    VoiceUtteranceDto,
  } from "../../lib/types";

  let status = $state<StatusResult | null>(null);
  let voice = $state<VoiceStatusDto | null>(null);
  let utterances = $state<VoiceUtteranceDto[]>([]);
  let configEdits = $state<DotfileEditDto[]>([]);
  let wakeEnabled = $state(true);
  let listenEnabled = $state(false);
  let language = $state<"en" | "pt">("en");
  let voiceTest = $state<string | null>(null);
  let revertBusy = $state<Set<string>>(new Set());
  let stop: (() => void) | null = null;

  async function load() {
    status = await optionalRpc<StatusResult | null>("status", null, null);
    voice = await optionalRpc<VoiceStatusDto | null>("voice.status", null, null);
    utterances = await optionalRpc<VoiceUtteranceDto[]>("voice.utterances", { limit: 6 }, []);
    configEdits = await optionalRpc<DotfileEditDto[]>("dotfile_edits.list", { limit: 6 }, []);
  }

  onMount(() => {
    stop = poll(load, 10000);
  });
  onDestroy(() => stop?.());

  function backendState(name: string): string {
    const backend = voice?.backends.find((b) => b.name === name);
    if (!backend) return "not reported";
    return backend.reason ? `${backend.state} · ${backend.reason}` : backend.state;
  }

  function testLanguage(lang: "en" | "pt") {
    language = lang;
    const wakeWord = lang === "pt" ? "ei rato" : "hey rat";
    const phrase = lang === "pt" ? "ei rato abre painel" : "hey rat open dashboard";
    voiceTest = `${wakeWord}: ${phrase}`;
  }

  async function revertEdit(id: string) {
    const next = new Set(revertBusy);
    next.add(id);
    revertBusy = next;
    try {
      await optionalRpc("dotfile_edits.revert", { id }, null);
      await load();
    } finally {
      const after = new Set(revertBusy);
      after.delete(id);
      revertBusy = after;
    }
  }

  function editSummary(edit: DotfileEditDto): string {
    const state = edit.reverted_by ? `reverted by ${edit.reverted_by.slice(0, 8)}` : "active";
    return `${edit.kind} · R${edit.risk} · ${state}`;
  }
</script>

<div class="col">
  <HudPanel title="Daemon">
    {#if status}
      <table>
        <tbody>
          <tr><td>version</td><td>ratd {status.version}</td></tr>
          <tr><td>protocol</td><td>v{status.proto_version}</td></tr>
          <tr><td>database</td><td>{status.db_path}</td></tr>
          <tr><td>events stored</td><td>{status.event_count}</td></tr>
        </tbody>
      </table>
    {:else}
      <span class="dim">daemon unreachable</span>
    {/if}
  </HudPanel>
  <HudPanel title="Milestones">
    <ul class="dim">
      <li>M3 — memory, retrieval, critic loop</li>
      <li>M4 — tmux workbench, worktrees, approvals</li>
      <li>M5 — screen OCR stubs, encrypted ring buffer, pins</li>
      <li>M6 — voice, wake words (rat / hey rat / rato / ei rato)</li>
    </ul>
  </HudPanel>
  <HudPanel title="Voice">
    <div class="voice-grid">
      <label class="toggle-row">
        <input type="checkbox" bind:checked={wakeEnabled} disabled={!voice?.enabled} />
        <span>wake word</span>
      </label>
      <label class="toggle-row">
        <input type="checkbox" bind:checked={listenEnabled} disabled={!voice?.enabled || !wakeEnabled} />
        <span>push to listen</span>
      </label>
      <div class="lang-row" role="group" aria-label="voice language test">
        <button class="hud-btn" class:active-lang={language === "en"} onclick={() => testLanguage("en")}>EN</button>
        <button class="hud-btn" class:active-lang={language === "pt"} onclick={() => testLanguage("pt")}>PT</button>
      </div>
      <div class="dim">
        pre-wake ring: {voice?.prewake_ring_secs ?? 8}s RAM-only
      </div>
    </div>

    <table class="voice-table">
      <tbody>
        <tr><td>mic</td><td>{backendState("mic")}</td></tr>
        <tr><td>wake</td><td>{backendState("wake")}</td></tr>
        <tr><td>vad</td><td>{backendState("vad")}</td></tr>
        <tr><td>stt</td><td>{backendState("stt")}</td></tr>
        <tr><td>tts</td><td>{backendState("tts")}</td></tr>
      </tbody>
    </table>

    {#if voiceTest}
      <div class="dim test-line">{voiceTest}</div>
    {/if}

    {#if utterances.length > 0}
      <div class="utterances">
        {#each utterances as u (u.id)}
          <div class="utterance">
            <span class="mono">{u.lang}</span>
            <span>{u.wake_word}</span>
            <span class="utterance-text">{u.text}</span>
          </div>
        {/each}
      </div>
    {:else}
      <div class="dim">no post-wake utterances stored</div>
    {/if}
  </HudPanel>

  <HudPanel title="Config Changes">
    {#if configEdits.length === 0}
      <div class="dim">no managed config edits yet</div>
    {:else}
      <div class="edit-list">
        {#each configEdits as edit (edit.id)}
          <div class="edit-row">
            <div class="edit-main">
              <div class="edit-path">{edit.path}</div>
              <div class="edit-meta">{editSummary(edit)} · {edit.reason}</div>
            </div>
            <button class="hud-btn edit-btn" disabled={revertBusy.has(edit.id)} onclick={() => revertEdit(edit.id)}>
              {revertBusy.has(edit.id) ? "…" : "Revert"}
            </button>
            <pre class="edit-diff">{edit.diff || "(no diff)"}</pre>
          </div>
        {/each}
      </div>
    {/if}
  </HudPanel>
</div>

<style>
  .col {
    display: flex;
    flex-direction: column;
    gap: 12px;
    max-width: 720px;
  }
  table {
    border-collapse: collapse;
    font-size: 12px;
  }
  td {
    padding: 3px 16px 3px 0;
  }
  td:first-child {
    color: var(--hud-ink-dim);
    font-family: var(--hud-font-head);
    font-size: 10px;
    text-transform: uppercase;
  }
  .dim {
    color: var(--hud-ink-dim);
    font-size: 12px;
  }
  ul {
    margin: 0;
    padding-left: 18px;
  }
  li {
    padding: 2px 0;
  }
  .voice-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px;
    align-items: center;
    margin-bottom: 12px;
  }
  .toggle-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-family: var(--hud-font-head);
    font-size: 11px;
    text-transform: uppercase;
    color: var(--hud-ink);
  }
  input[type="checkbox"] {
    width: 16px;
    height: 16px;
    accent-color: var(--hud-accent);
  }
  input:disabled {
    opacity: 0.55;
  }
  .lang-row {
    display: flex;
    gap: 6px;
  }
  .lang-row .hud-btn {
    min-width: 42px;
    padding: 3px 10px;
    font-size: 11px;
  }
  .active-lang {
    color: var(--hud-ok);
    border-color: var(--hud-ok);
  }
  .voice-table {
    margin-bottom: 10px;
  }
  .test-line {
    margin-bottom: 8px;
    font-family: var(--hud-font-data);
  }
  .utterances {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .utterance {
    display: grid;
    grid-template-columns: 34px 72px minmax(0, 1fr);
    gap: 8px;
    align-items: center;
    font-size: 12px;
    border-top: 1px solid color-mix(in srgb, var(--hud-ink) 12%, transparent);
    padding-top: 5px;
  }
  .utterance-text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mono {
    font-family: var(--hud-font-data);
    color: var(--hud-ink-dim);
  }
  .edit-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .edit-row {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 6px 10px;
    padding-top: 8px;
    border-top: 1px solid color-mix(in srgb, var(--hud-ink) 14%, transparent);
  }
  .edit-main {
    min-width: 0;
  }
  .edit-path {
    font-family: var(--hud-font-head);
    font-size: 12px;
    color: var(--hud-ink);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .edit-meta {
    font-family: var(--hud-font-data);
    font-size: 11px;
    color: var(--hud-ink-dim);
  }
  .edit-btn {
    align-self: start;
    font-size: 11px;
    padding: 3px 8px;
  }
  .edit-diff {
    grid-column: 1 / -1;
    margin: 0;
    padding: 8px 10px;
    background: color-mix(in srgb, var(--hud-ink) 5%, var(--hud-panel));
    border: 1px solid color-mix(in srgb, var(--hud-ink) 14%, transparent);
    font-family: var(--hud-font-data);
    font-size: 11px;
    line-height: 1.4;
    color: var(--hud-ink);
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 120px;
    overflow: auto;
  }
</style>
