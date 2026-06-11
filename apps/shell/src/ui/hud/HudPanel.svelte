<script lang="ts">
  import type { Snippet } from "svelte";
  let { title, children }: { title?: string; children: Snippet } = $props();
</script>

<div class="hud-panel hud-dither">
  {#if title}
    <div class="hud-title">{title}</div>
  {/if}
  <div class="content">
    {@render children()}
  </div>
</div>

<style>
  .content {
    padding: 8px;
  }
</style>
