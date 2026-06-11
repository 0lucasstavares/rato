<script lang="ts">
  let {
    label,
    value,
    max,
    color = "var(--hud-accent)",
    text = "",
  }: { label: string; value: number; max: number; color?: string; text?: string } = $props();

  let pct = $derived(Math.max(0, Math.min(100, (value / Math.max(1, max)) * 100)));
</script>

<div class="meter">
  <span class="label">{label}</span>
  <span class="bar">
    <span class="fill" style:width="{pct}%" style:background={color}></span>
  </span>
  <span class="text">{text}</span>
</div>

<style>
  .meter {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 4px 0;
  }
  .label {
    font-family: var(--hud-font-head);
    font-size: 10px;
    letter-spacing: 1px;
    text-transform: uppercase;
    width: 90px;
    color: var(--hud-ink-dim);
  }
  .bar {
    display: inline-block;
    width: 150px;
    height: 12px;
    border: 2px solid var(--hud-ink);
    background: var(--hud-panel);
    box-shadow: var(--hud-shadow-sm);
  }
  .fill {
    display: block;
    height: 100%;
    /* marker stroke: slightly ragged right edge */
    clip-path: polygon(0 0, 100% 0, 97% 55%, 100% 100%, 0 100%);
  }
  .text {
    font-size: 12px;
    font-family: var(--hud-font-data);
    color: var(--hud-ink-dim);
  }
</style>
