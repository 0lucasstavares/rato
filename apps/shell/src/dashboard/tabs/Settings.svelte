<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import HudPanel from "../../ui/hud/HudPanel.svelte";
  import { poll, rpc } from "../../lib/rpc";
  import type { StatusResult } from "../../lib/types";

  let status = $state<StatusResult | null>(null);
  let stop: (() => void) | null = null;

  onMount(() => {
    stop = poll(async () => {
      status = await rpc<StatusResult>("status");
    }, 10000);
  });
  onDestroy(() => stop?.());
</script>

<div class="col">
  <HudPanel title="Daemon (read-only in M2)">
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
  <HudPanel title="Coming up">
    <ul class="dim">
      <li>M3 — memory, retrieval, critic loop (LLM provider: OpenAI / Anthropic / OpenRouter)</li>
      <li>M4 — tmux workbench, worktrees, approvals</li>
      <li>M5 — screen OCR + encrypted ring buffer</li>
      <li>M6 — voice, wake words (rat / hey rat / rato / ei rato)</li>
    </ul>
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
    color: var(--hud-text-dim);
    font-family: var(--hud-font-head);
    font-size: 10px;
    text-transform: uppercase;
  }
  .dim {
    color: var(--hud-text-dim);
    font-size: 12px;
  }
  ul {
    margin: 0;
    padding-left: 18px;
  }
  li {
    padding: 2px 0;
  }
</style>
