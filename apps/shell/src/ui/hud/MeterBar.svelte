<script lang="ts">
  let {
    label,
    value,
    max,
    color = "var(--hud-accent)",
    text = "",
  }: { label: string; value: number; max: number; color?: string; text?: string } = $props();

  const SEGMENTS = 20;
  let filled = $derived(
    Math.max(0, Math.min(SEGMENTS, Math.round((value / Math.max(1, max)) * SEGMENTS))),
  );
</script>

<div class="meter">
  <span class="label">{label}</span>
  <span class="segments">
    {#each Array(SEGMENTS) as _, i}
      <span class="seg" style:background={i < filled ? color : "color-mix(in srgb, var(--hud-ink) 20%, transparent)"}></span>
    {/each}
  </span>
  <span class="text">{text}</span>
</div>

<style>
  .meter {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 3px 0;
  }
  .label {
    font-family: var(--hud-font-head);
    font-size: 9px;
    letter-spacing: 1px;
    text-transform: uppercase;
    width: 90px;
    color: var(--hud-ink-dim);
  }
  .segments {
    display: inline-flex;
    gap: 2px;
    border: 1px solid var(--hud-ink);
    padding: 2px;
    background: var(--hud-bg);
  }
  .seg {
    width: 7px;
    height: 10px;
    display: inline-block;
  }
  .text {
    font-size: 11px;
    color: var(--hud-ink-dim);
  }
</style>
