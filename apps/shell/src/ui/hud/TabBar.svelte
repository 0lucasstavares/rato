<script lang="ts">
  let {
    tabs,
    active = $bindable(),
  }: { tabs: string[]; active: string } = $props();
</script>

<nav class="tabbar">
  {#each tabs as tab}
    <button
      class="tab"
      class:hud-marker-stroke={active === tab}
      class:active={active === tab}
      onclick={() => (active = tab)}
    >
      {tab}
    </button>
  {/each}
</nav>

<style>
  .tabbar {
    display: flex;
    gap: 18px;
    padding: 10px 16px 12px;
  }
  .tab {
    font-family: var(--hud-font-head);
    font-size: 17px;
    letter-spacing: 1px;
    text-transform: uppercase;
    color: var(--hud-ink-dim);
    background: none;
    border: none;
    padding: 0;
    cursor: pointer;
  }
  .tab:hover {
    color: var(--hud-ink);
    rotate: -1deg;
  }
  .tab.active {
    color: var(--hud-ink);
  }
</style>
