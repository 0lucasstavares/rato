<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    title,
    body,
    footer = null,
    tone = "info",
    actionLabel = null,
    onAction = null,
    children,
  }: {
    title: string;
    body: string;
    footer?: string | null;
    tone?: "info" | "warn" | "danger" | "ok";
    actionLabel?: string | null;
    onAction?: (() => void | Promise<void>) | null;
    children?: Snippet;
  } = $props();

  async function trigger() {
    await onAction?.();
  }
</script>

<div class={"dialogue hud-panel hud-tape hud-grunge tone-" + tone}>
  <div class="content">
    <div class="topline">
      <span class="eyebrow">dialogue</span>
      <span class="tone">{tone}</span>
    </div>
    <div class="title">{title}</div>
    <div class="body">{body}</div>
    {#if footer}
      <div class="footer">{footer}</div>
    {/if}
    {#if actionLabel}
      <button class="hud-btn action" onclick={trigger}>{actionLabel}</button>
    {/if}
    {#if children}
      <div class="children">{@render children()}</div>
    {/if}
  </div>
</div>

<style>
  .dialogue {
    border-left: 5px solid var(--hud-info);
  }
  .dialogue.tone-warn {
    border-left-color: var(--hud-warn);
  }
  .dialogue.tone-danger {
    border-left-color: var(--hud-danger);
  }
  .dialogue.tone-ok {
    border-left-color: var(--hud-ok);
  }
  .content {
    padding: 8px 10px 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .topline {
    display: flex;
    align-items: center;
    gap: 10px;
    font-family: var(--hud-font-head);
    font-size: 10px;
    letter-spacing: 1px;
    text-transform: uppercase;
    color: var(--hud-ink-dim);
  }
  .title {
    font-family: var(--hud-font-head);
    font-size: 16px;
    line-height: 1.1;
    color: var(--hud-ink);
  }
  .body {
    font-family: var(--hud-font-body);
    font-size: 13px;
    line-height: 1.45;
    color: var(--hud-ink);
  }
  .footer {
    font-family: var(--hud-font-data);
    font-size: 11px;
    color: var(--hud-ink-dim);
  }
  .action {
    align-self: flex-start;
  }
  .children {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
</style>
