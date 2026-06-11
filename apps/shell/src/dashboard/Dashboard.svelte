<script lang="ts">
  import TabBar from "../ui/hud/TabBar.svelte";
  import Now from "./tabs/Now.svelte";
  import Sensors from "./tabs/Sensors.svelte";
  import Settings from "./tabs/Settings.svelte";

  let active = $state("Now");
</script>

<div class="dash">
  <header class="hud-panel hud-scanlines">
    <span class="logo">RATO</span>
    <span class="sub">developer companion · M2 shell</span>
  </header>
  <TabBar tabs={["Now", "Sensors", "Settings"]} bind:active />
  <main>
    {#if active === "Now"}
      <Now />
    {:else if active === "Sensors"}
      <Sensors />
    {:else}
      <Settings />
    {/if}
  </main>
</div>

<style>
  .dash {
    display: flex;
    flex-direction: column;
    height: 100vh;
  }
  header {
    display: flex;
    align-items: baseline;
    gap: 12px;
    padding: 10px 14px;
    margin: 8px;
  }
  .logo {
    font-family: var(--hud-font-head);
    font-size: 22px;
    letter-spacing: 4px;
    color: var(--hud-accent);
  }
  .sub {
    font-size: 11px;
    color: var(--hud-text-dim);
  }
  main {
    flex: 1;
    overflow: auto;
    padding: 12px;
  }
</style>
