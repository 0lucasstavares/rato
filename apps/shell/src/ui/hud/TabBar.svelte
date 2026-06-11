<script lang="ts">
  let {
    tabs,
    active = $bindable(),
  }: { tabs: string[]; active: string } = $props();
</script>

<nav class="tabbar">
  {#each tabs as tab}
    <button
      class="hud-btn tab"
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
    gap: 4px;
    padding: 8px 8px 0 8px;
    border-bottom: 2px solid var(--hud-line);
  }
  .tab {
    border-bottom: none;
  }
  .tab.active {
    color: var(--hud-accent);
    border-color: var(--hud-accent);
    border-bottom: none;
    background: var(--hud-panel);
  }
</style>
