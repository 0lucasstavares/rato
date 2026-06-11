<script lang="ts">
  let {
    label,
    state = "off",
  }: { label: string; state?: "on" | "off" | "warn" | "err" } = $props();
</script>

<span class="hud-chip {state}" title="{label}: {state}">
  <span class="dot"></span>{label}
</span>
