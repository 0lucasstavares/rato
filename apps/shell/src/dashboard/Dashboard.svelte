<script lang="ts">
  import TabBar from "../ui/hud/TabBar.svelte";
  import Now from "./tabs/Now.svelte";
  import Sensors from "./tabs/Sensors.svelte";
  import Settings from "./tabs/Settings.svelte";
  import Pushback from "./tabs/Pushback.svelte";
  import Agents from "./tabs/Agents.svelte";
  import Workbench from "./tabs/Workbench.svelte";
  import Approvals from "./tabs/Approvals.svelte";
  import Pins from "./tabs/Pins.svelte";
  import Calendar from "./tabs/Calendar.svelte";
  import Metrics from "./tabs/Metrics.svelte";
  import Memory from "./tabs/Memory.svelte";

  let active = $state("Now");
</script>

<div class="dash hud-grunge">
  <header>
    <span class="logo">RATO</span>
    <span class="sub">developer companion ?? M7 shell</span>
  </header>
  <TabBar tabs={["Now", "Calendar", "Metrics", "Memory", "Pushback", "Agents", "Workbench", "Approvals", "Pins", "Sensors", "Settings"]} bind:active />
  <main>
    {#if active === "Now"}
      <Now />
    {:else if active === "Calendar"}
      <Calendar />
    {:else if active === "Metrics"}
      <Metrics />
    {:else if active === "Memory"}
      <Memory />
    {:else if active === "Pushback"}
      <Pushback />
    {:else if active === "Agents"}
      <Agents />
    {:else if active === "Workbench"}
      <Workbench />
    {:else if active === "Approvals"}
      <Approvals />
    {:else if active === "Pins"}
      <Pins />
    {:else if active === "Sensors"}
      <Sensors />
    {:else}
      <Settings />
    {/if}
  </main>
  <div class="watermark" aria-hidden="true">RATO</div>
</div>

<style>
  .dash {
    display: flex;
    flex-direction: column;
    height: 100vh;
    position: relative;
    overflow: hidden;
  }
  header {
    display: flex;
    align-items: baseline;
    gap: 14px;
    padding: 14px 16px 0;
  }
  .logo {
    font-family: var(--hud-font-head);
    font-size: 34px;
    line-height: 1;
    letter-spacing: 2px;
    color: var(--hud-ink);
    text-shadow: 3px 3px 0 var(--hud-accent);
    rotate: -1.5deg;
  }
  .sub {
    font-family: var(--hud-font-marker);
    font-size: 13px;
    color: var(--hud-ink-dim);
  }
  main {
    flex: 1;
    overflow: auto;
    padding: 4px 16px 16px;
    z-index: 1;
  }
  .watermark {
    position: absolute;
    bottom: -30px;
    right: -20px;
    font-family: var(--hud-font-head);
    font-size: 160px;
    line-height: 1;
    color: var(--hud-ink);
    opacity: 0.05;
    rotate: -8deg;
    pointer-events: none;
    user-select: none;
  }
</style>

