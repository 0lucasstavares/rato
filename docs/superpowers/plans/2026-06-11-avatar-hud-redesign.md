# Biped Rat Avatar + THUG2 HUD Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild the procedural avatar as a front-facing biped rat bust (torso-up, hands) anchored to the screen bottom, and restyle the entire HUD from dark-terminal to THUG2 skate-zine (cream paper, stickers, tape, spray orange).

**Architecture:** Spec at `docs/superpowers/specs/2026-06-11-avatar-hud-redesign-design.md`. All texture is CSS/SVG (no image assets); fonts are OFL woff2 files committed to the repo; the rat stays a flat-shaded Three.js procedural placeholder (M7 replaces it with glTF). Window shrinks to 180×240 and sits flush with the screen bottom.

**Tech Stack:** Svelte 5 + Vite (apps/shell), Three.js, Tauri 2 (src-tauri, built with plain cargo — `custom-protocol` feature already on), Python/PIL for the icon.

**Verification commands** (no JS unit-test infra in apps/shell; each task gates on these):
- `cd ~/rato/apps/shell && export PATH="$HOME/.local/bin:$PATH" && npm run check` → expect `0 errors`
- `npm run build` → expect `✓ built`
- Rust: `export PATH="$HOME/.cargo/bin:$PATH" && cargo build --release --manifest-path ~/rato/apps/shell/src-tauri/Cargo.toml`

Working directory for all tasks: `~/rato/apps/shell` unless stated.

---

### Task 1: Bundle webfonts (Anton, Permanent Marker, Barlow)

**Files:**
- Create: `apps/shell/src/assets/fonts/anton.woff2`, `permanent-marker.woff2`, `barlow-400.woff2`, `barlow-600.woff2`
- Create: `apps/shell/src/assets/fonts/OFL.txt`

- [ ] **Step 1: Download woff2 files from Google Fonts (latin subset = last url in each css response)**

```bash
cd ~/rato/apps/shell && mkdir -p src/assets/fonts
UA="Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36"
geturl() { curl -sA "$UA" "https://fonts.googleapis.com/css2?family=$1" | grep -o 'https://[^)]*\.woff2' | tail -1; }
curl -sL "$(geturl 'Anton')" -o src/assets/fonts/anton.woff2
curl -sL "$(geturl 'Permanent+Marker')" -o src/assets/fonts/permanent-marker.woff2
curl -sL "$(geturl 'Barlow:wght@400')" -o src/assets/fonts/barlow-400.woff2
curl -sL "$(geturl 'Barlow:wght@600')" -o src/assets/fonts/barlow-600.woff2
file src/assets/fonts/*.woff2
```

Expected: `file` reports each as `Web Open Font Format (Version 2)`. If network fails, STOP and report — fallback stacks exist but bundling is the spec'd path.

- [ ] **Step 2: Write the license pointer**

`src/assets/fonts/OFL.txt`:
```
Anton, Permanent Marker, Barlow — SIL Open Font License 1.1.
Sources: https://fonts.google.com/specimen/Anton , /specimen/Permanent+Marker , /specimen/Barlow
Full license text: https://openfontlicense.org/open-font-license-official-text/
```

- [ ] **Step 3: Commit**

```bash
git add src/assets/fonts && git commit -m "feat(shell): bundle OFL webfonts (Anton, Permanent Marker, Barlow)"
```

---

### Task 2: Rewrite tokens.css — THUG2 design tokens

**Files:**
- Modify: `apps/shell/src/ui/hud/tokens.css` (full replacement)

- [ ] **Step 1: Replace the entire file with:**

```css
/* HUD-THUG2 design system — spec: docs/superpowers/specs/2026-06-11-avatar-hud-redesign-design.md
   Skate-zine collage: cream paper, ink, die-cut stickers, tape, spray orange.
   No image assets: grunge is an inline SVG feTurbulence data-URI. */

@font-face {
  font-family: "Anton";
  src: url("../../assets/fonts/anton.woff2") format("woff2");
  font-display: swap;
}
@font-face {
  font-family: "Permanent Marker";
  src: url("../../assets/fonts/permanent-marker.woff2") format("woff2");
  font-display: swap;
}
@font-face {
  font-family: "Barlow";
  src: url("../../assets/fonts/barlow-400.woff2") format("woff2");
  font-weight: 400;
  font-display: swap;
}
@font-face {
  font-family: "Barlow";
  src: url("../../assets/fonts/barlow-600.woff2") format("woff2");
  font-weight: 600;
  font-display: swap;
}

:root {
  --hud-bg: #e9e1cb;
  --hud-panel: #f7f1e2;
  --hud-ink: #1e1a15;
  --hud-ink-dim: #6b6256;
  --hud-accent: #f26b1d;
  --hud-ok: #3fa544;
  --hud-warn: #f2b705;
  --hud-danger: #d23b2e;
  --hud-info: #2a6fc8;
  --hud-tape: rgba(244, 238, 210, 0.75);

  --hud-font-head: "Anton", "Arial Narrow", Impact, sans-serif;
  --hud-font-marker: "Permanent Marker", "Comic Sans MS", cursive;
  --hud-font-body: "Barlow", system-ui, sans-serif;
  --hud-font-data: "IBM Plex Mono", "JetBrains Mono", monospace;

  --hud-shadow: 4px 4px 0 var(--hud-ink);
  --hud-shadow-sm: 2px 2px 0 var(--hud-ink);
}

.hud-body {
  background: var(--hud-bg);
  color: var(--hud-ink);
  font-family: var(--hud-font-body);
  font-size: 14px;
  margin: 0;
  min-height: 100vh;
}

/* paper grain — shared grunge overlay (feTurbulence noise) */
.hud-grunge {
  position: relative;
}
.hud-grunge::after {
  content: "";
  position: absolute;
  inset: 0;
  pointer-events: none;
  opacity: 0.05;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='160' height='160'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.9' numOctaves='2'/%3E%3C/filter%3E%3Crect width='160' height='160' filter='url(%23n)'/%3E%3C/svg%3E");
}

/* paper card: hard shadow, ink border, slight alternating tilt */
.hud-panel {
  background: var(--hud-panel);
  border: 2px solid var(--hud-ink);
  box-shadow: var(--hud-shadow);
  position: relative;
  rotate: -0.5deg;
}
.hud-panel:nth-child(even) {
  rotate: 0.6deg;
}

/* tape strips on two corners of a card */
.hud-tape::before,
.hud-tape::after {
  content: "";
  position: absolute;
  width: 56px;
  height: 18px;
  background: var(--hud-tape);
  border: 1px solid rgba(30, 26, 21, 0.15);
  pointer-events: none;
}
.hud-tape::before {
  top: -9px;
  left: 14px;
  transform: rotate(-6deg);
}
.hud-tape::after {
  bottom: -9px;
  right: 14px;
  transform: rotate(5deg);
}

.hud-title {
  font-family: var(--hud-font-marker);
  font-size: 14px;
  padding: 4px 10px 0;
  color: var(--hud-ink);
}

/* die-cut sticker chip */
.hud-chip {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-family: var(--hud-font-head);
  font-size: 10px;
  letter-spacing: 1px;
  padding: 2px 7px;
  color: var(--hud-ink);
  background: var(--hud-panel);
  border: 3px solid #fff;
  outline: 1px solid rgba(30, 26, 21, 0.35);
  box-shadow: var(--hud-shadow-sm);
  text-transform: uppercase;
  rotate: -1deg;
}
.hud-chip:nth-child(even) {
  rotate: 1.2deg;
}

.hud-chip .dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  border: 1px solid var(--hud-ink);
  display: inline-block;
}
.hud-chip.on .dot { background: var(--hud-ok); }
.hud-chip.off .dot { background: var(--hud-ink-dim); }
.hud-chip.warn .dot { background: var(--hud-warn); }
.hud-chip.err .dot { background: var(--hud-danger); }

/* sticker button */
button.hud-btn {
  font-family: var(--hud-font-head);
  font-size: 12px;
  letter-spacing: 1px;
  text-transform: uppercase;
  color: var(--hud-ink);
  background: var(--hud-panel);
  border: 2px solid var(--hud-ink);
  box-shadow: var(--hud-shadow-sm);
  padding: 5px 12px;
  cursor: pointer;
}
button.hud-btn:hover {
  rotate: -1deg;
  color: var(--hud-accent);
}
button.hud-btn:active {
  transform: translate(2px, 2px);
  box-shadow: none;
}

/* rough orange marker underline (active tab, emphasis) */
.hud-marker-stroke {
  position: relative;
}
.hud-marker-stroke::after {
  content: "";
  position: absolute;
  left: -3px;
  right: -5px;
  bottom: -4px;
  height: 5px;
  background: var(--hud-accent);
  transform: rotate(-1.2deg) skewX(-12deg);
}

@keyframes hud-blink-steps {
  0%, 49% { opacity: 1; }
  50%, 100% { opacity: 0.25; }
}
```

- [ ] **Step 2: Fix old-token references in component-local styles (mechanical sweep)**

These are the only files using removed tokens/classes (`--hud-text*`, `--hud-line*`, `hud-scanlines`, `hud-dither`). Apply exactly:

- `src/dashboard/Dashboard.svelte:11` — change `class="hud-panel hud-scanlines"` → `class="hud-panel hud-tape"`, and `:48` `var(--hud-text-dim)` → `var(--hud-ink-dim)`
- `src/dashboard/tabs/Now.svelte:77,96` — `var(--hud-text-dim)` → `var(--hud-ink-dim)`; `:103` `var(--hud-line-dark)` → `color-mix(in srgb, var(--hud-ink) 20%, transparent)`
- `src/dashboard/tabs/Settings.svelte:58,64` — `var(--hud-text-dim)` → `var(--hud-ink-dim)`
- `src/dashboard/tabs/Sensors.svelte:104` — `var(--hud-text-dim)` → `var(--hud-ink-dim)`; `:109` `var(--hud-line-dark)` → `color-mix(in srgb, var(--hud-ink) 20%, transparent)`
- `src/avatar/Avatar.svelte:89` — `class="quick hud-panel hud-dither"` → `class="quick hud-panel"` (full restyle in Task 5); `:111` `var(--hud-line)` → `var(--hud-ink)`; `:139` `var(--hud-text)` → `var(--hud-ink)`; `:143` `var(--hud-text-dim)` → `var(--hud-ink-dim)`
- `src/ui/hud/HudPanel.svelte` — change `class="hud-panel hud-dither"` → `class="hud-panel"` (Task 3 finishes it)

(Line numbers are as of commit `30cebec`; match on content if drifted.)

- [ ] **Step 3: Verify**

Run: `npm run check && npm run build`
Expected: 0 errors, build succeeds.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(shell): THUG2 design tokens — paper/sticker/tape, webfonts, kill scanlines"
```

---

### Task 3: Restyle HudPanel + StatusChip

**Files:**
- Modify: `apps/shell/src/ui/hud/HudPanel.svelte` (full replacement)
- Modify: `apps/shell/src/ui/hud/StatusChip.svelte` (no change beyond tokens — verify only)

- [ ] **Step 1: Replace `HudPanel.svelte` with:**

```svelte
<script lang="ts">
  import type { Snippet } from "svelte";
  let { title, children }: { title?: string; children: Snippet } = $props();
</script>

<div class="hud-panel hud-tape hud-grunge">
  {#if title}
    <div class="hud-title">{title}</div>
  {/if}
  <div class="content">
    {@render children()}
  </div>
</div>

<style>
  .content {
    padding: 8px 10px 10px;
  }
</style>
```

- [ ] **Step 2: StatusChip** — markup already consumes `.hud-chip`/`.dot` classes restyled in Task 2; confirm no local styles reference removed tokens (there are none). No edit.

- [ ] **Step 3: Verify**

Run: `npm run check && npm run build` — expected green.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(shell): HudPanel as taped paper card"
```

---

### Task 4: Restyle TabBar + MeterBar

**Files:**
- Modify: `apps/shell/src/ui/hud/TabBar.svelte` (full replacement)
- Modify: `apps/shell/src/ui/hud/MeterBar.svelte` (full replacement)

- [ ] **Step 1: Replace `TabBar.svelte` with:**

```svelte
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
```

- [ ] **Step 2: Replace `MeterBar.svelte` with:**

```svelte
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
```

- [ ] **Step 3: Verify**

Run: `npm run check && npm run build` — expected green.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(shell): THUG2 tabs (marker underline) + marker-fill meter"
```

---

### Task 5: Dashboard chrome + avatar overlays

**Files:**
- Modify: `apps/shell/src/dashboard/Dashboard.svelte`
- Modify: `apps/shell/src/avatar/Avatar.svelte` (styles + layout only; rat canvas rebuilt in Task 6)
- Modify: `apps/shell/avatar.html`

- [ ] **Step 1: Dashboard.svelte — replace the markup `<header>` block and `<style>` with:**

```svelte
<div class="dash hud-grunge">
  <header>
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
  <div class="watermark" aria-hidden="true">RATO</div>
</div>
```

```css
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
```

(Script block unchanged. The header is no longer a `.hud-panel` — it's ink-on-paper with an orange offset shadow, THUG2 title style.)

- [ ] **Step 2: Avatar.svelte — replace the layout/styles (keep script logic, polling, handlers identical):**

Markup:
```svelte
<div class="avatar-root">
  <div
    class="grip"
    onmousedown={startDrag}
    role="toolbar"
    tabindex="-1"
    title="drag to move"
  >
    <StatusChip label="SCR" state="off" />
    <StatusChip label="MIC" state="off" />
    <StatusChip label="CLP" state={net === "on" ? "on" : "off"} />
    <StatusChip label="NET" state={net === "on" ? "on" : "err"} />
  </div>

  <canvas
    bind:this={canvas}
    class="rat"
    width="150"
    height="180"
    onclick={onBodyClick}
    ondblclick={openDashboard}
    oncontextmenu={onContextMenu}
  ></canvas>

  {#if mode.mode === "away"}
    <div class="zzz">z Z z</div>
  {/if}

  {#if showQuick}
    <div class="quick hud-panel">
      <button class="hud-btn" onclick={openDashboard}>Dash</button>
      <button class="hud-btn" onclick={() => (showQuick = false)}>×</button>
      <div class="hint">{mode.mode}{mode.idle_ms !== null ? ` · idle ${Math.floor(mode.idle_ms / 1000)}s` : ""}</div>
    </div>
  {/if}
</div>
```

Styles:
```css
  .avatar-root {
    width: 180px;
    height: 240px;
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
  }
  /* tape band across the top: drag handle + sticker LEDs */
  .grip {
    display: flex;
    gap: 2px;
    justify-content: center;
    padding: 3px 0;
    width: 170px;
    background: var(--hud-tape);
    border: 1px solid rgba(30, 26, 21, 0.2);
    transform: rotate(-1.2deg);
    cursor: grab;
    z-index: 2;
  }
  .grip :global(.hud-chip) {
    font-size: 7px;
    padding: 1px 3px;
    border-width: 2px;
    box-shadow: none;
  }
  .rat {
    width: 180px;
    height: 216px;
    margin-top: auto;
    image-rendering: pixelated;
    cursor: pointer;
  }
  .zzz {
    position: absolute;
    top: 50px;
    right: 18px;
    font-family: var(--hud-font-marker);
    font-size: 15px;
    color: var(--hud-info);
    text-shadow: 1px 1px 0 #fff;
    animation: hud-blink-steps 2s steps(2) infinite;
    pointer-events: none;
  }
  .quick {
    position: absolute;
    bottom: 10px;
    left: 50%;
    transform: translateX(-50%) rotate(-1deg);
    display: flex;
    gap: 6px;
    align-items: center;
    padding: 7px;
    color: var(--hud-ink);
    z-index: 3;
  }
  .quick .hint {
    font-family: var(--hud-font-marker);
    font-size: 10px;
    color: var(--hud-ink-dim);
    white-space: nowrap;
  }
```

Canvas internal resolution is now 150×180 (5:6), CSS 180×216 — Task 6's renderer matches.

- [ ] **Step 3: avatar.html — body stays transparent (no change needed); confirm the inline `<style>` still sets `background: transparent` and nothing references removed classes.**

- [ ] **Step 4: Verify**

Run: `npm run check && npm run build` — expected green.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(shell): dashboard THUG2 chrome + avatar tape-grip layout (180x240)"
```

---

### Task 6: Rebuild rat3d.ts — biped torso-up rat with hands

**Files:**
- Modify: `apps/shell/src/avatar/rat3d.ts` (full replacement)

- [ ] **Step 1: Replace the entire file with:**

```typescript
import * as THREE from "three";

/**
 * Procedural low-poly PS2-style rat (M2 placeholder for the Blender glTF
 * model that arrives in M7). Biped bust, framed torso-up: the canvas bottom
 * edge crops him at the waist, so he rises out of the screen bottom.
 * Flat-shaded primitives, rendered low-res and upscaled nearest-neighbor.
 */
export interface Rat3D {
  setMode(mode: "active" | "away"): void;
  dispose(): void;
}

const FUR = 0x8d93a1;
const FUR_DARK = 0x6c7280;
const PINK = 0xd99aa8;
const EYE = 0x16181d;

function flat(color: number): THREE.MeshLambertMaterial {
  return new THREE.MeshLambertMaterial({ color, flatShading: true });
}

interface RatParts {
  group: THREE.Group;
  head: THREE.Group;
  earL: THREE.Mesh;
  earR: THREE.Mesh;
  eyes: THREE.Mesh[];
  armL: THREE.Group;
  armR: THREE.Group;
}

/** One arm: pivot at the shoulder, cylinder hanging down, mitt hand at the end. */
function buildArm(side: 1 | -1): THREE.Group {
  const arm = new THREE.Group();
  const upper = new THREE.Mesh(new THREE.CylinderGeometry(0.13, 0.11, 0.8, 5), flat(FUR_DARK));
  upper.position.y = -0.4;
  arm.add(upper);

  const hand = new THREE.Group();
  const palm = new THREE.Mesh(new THREE.IcosahedronGeometry(0.16, 0), flat(PINK));
  palm.scale.set(1, 1.15, 0.8);
  hand.add(palm);
  const thumb = new THREE.Mesh(new THREE.BoxGeometry(0.07, 0.16, 0.07), flat(PINK));
  thumb.position.set(side * -0.14, 0.02, 0.06);
  thumb.rotation.z = side * 0.5;
  hand.add(thumb);
  hand.position.y = -0.88;
  arm.add(hand);
  return arm;
}

function buildRat(): RatParts {
  const g = new THREE.Group();

  // torso: plump pear — hips sphere (partially below the crop line) + chest
  const hips = new THREE.Mesh(new THREE.IcosahedronGeometry(1.0, 0), flat(FUR));
  hips.scale.set(1.15, 1.0, 0.85);
  hips.position.y = -1.15;
  g.add(hips);

  const chest = new THREE.Mesh(new THREE.IcosahedronGeometry(0.75, 0), flat(FUR));
  chest.scale.set(0.95, 0.9, 0.78);
  chest.position.y = -0.3;
  g.add(chest);

  // belly patch
  const belly = new THREE.Mesh(new THREE.IcosahedronGeometry(0.62, 0), flat(0xb9bec9));
  belly.scale.set(0.72, 0.85, 0.5);
  belly.position.set(0, -0.85, 0.55);
  g.add(belly);

  // head group (head + snout + nose + ears + eyes) — pivots for the away slump
  const head = new THREE.Group();
  head.position.y = 0.42; // neck joint

  const skull = new THREE.Mesh(new THREE.IcosahedronGeometry(0.62, 1), flat(FUR));
  skull.position.y = 0.3;
  head.add(skull);

  // snout: blunt cone pointing at the camera
  const snout = new THREE.Mesh(new THREE.ConeGeometry(0.3, 0.5, 6), flat(FUR));
  snout.rotation.x = Math.PI / 2;
  snout.position.set(0, 0.16, 0.62);
  head.add(snout);

  const nose = new THREE.Mesh(new THREE.IcosahedronGeometry(0.09, 0), flat(PINK));
  nose.position.set(0, 0.16, 0.9);
  head.add(nose);

  // ears: big discs facing the camera, pink inner / grey rim
  const earRim = new THREE.CylinderGeometry(0.36, 0.36, 0.07, 8);
  const earInner = new THREE.CylinderGeometry(0.26, 0.26, 0.08, 8);
  const earL = new THREE.Mesh(earRim, flat(FUR_DARK));
  earL.rotation.x = Math.PI / 2;
  earL.rotation.z = 0.22;
  earL.position.set(-0.52, 0.85, -0.05);
  const innerL = new THREE.Mesh(earInner, flat(PINK));
  innerL.position.y = 0.01; // proud of the rim, toward the camera after rotation
  earL.add(innerL);
  const earR = earL.clone();
  earR.position.x = 0.52;
  earR.rotation.z = -0.22;
  head.add(earL, earR);

  // eyes: black beads, front of the skull
  const eyeGeo = new THREE.IcosahedronGeometry(0.09, 0);
  const eyeL = new THREE.Mesh(eyeGeo, flat(EYE));
  eyeL.position.set(-0.24, 0.42, 0.5);
  const eyeR = eyeL.clone();
  eyeR.position.x = 0.24;
  head.add(eyeL, eyeR);

  g.add(head);

  // arms at the shoulders, hanging relaxed
  const armL = buildArm(-1);
  armL.position.set(-0.72, 0.05, 0.05);
  armL.rotation.z = -0.18;
  const armR = buildArm(1);
  armR.position.set(0.72, 0.05, 0.05);
  armR.rotation.z = 0.18;
  g.add(armL, armR);

  return { group: g, head, earL, earR, eyes: [eyeL, eyeR], armL, armR };
}

export function mountRat(canvas: HTMLCanvasElement): Rat3D {
  // render low, upscale nearest → PS2 chunk; 5:6 matches the 180×216 css box
  const RES_W = 150;
  const RES_H = 180;
  const renderer = new THREE.WebGLRenderer({ canvas, alpha: true, antialias: false });
  renderer.setSize(RES_W, RES_H, false);
  renderer.setPixelRatio(1);

  const scene = new THREE.Scene();
  // visible height ≈ 3.25 world units at z=0; crop line at y≈-1.6
  const camera = new THREE.PerspectiveCamera(36, RES_W / RES_H, 0.1, 50);
  camera.position.set(0, 0.05, 5.0);
  camera.lookAt(0, 0.05, 0);

  scene.add(new THREE.AmbientLight(0xb0bcd0, 1.1));
  const key = new THREE.DirectionalLight(0xfff4dd, 1.6);
  key.position.set(2, 4, 3);
  scene.add(key);

  const { group, head, earL, earR, eyes, armL, armR } = buildRat();
  scene.add(group);

  let mode: "active" | "away" = "active";
  let disposed = false;
  let blinkUntil = 0;
  let nextBlink = performance.now() + 2500;
  // rare idle flourish: raise the right hand in a small wave
  let waveStart = -1;
  let nextWave = performance.now() + 15000 + Math.random() * 25000;
  const WAVE_MS = 2200;
  const ARM_REST = 0.18;

  function animate(now: number) {
    if (disposed) return;
    requestAnimationFrame(animate);
    const t = now / 1000;
    // idle motion quantized to 12 fps for the retro feel
    const qt = Math.floor(t * 12) / 12;

    if (mode === "active") {
      // breathing bob + slight sway
      group.position.y = Math.sin(qt * 2.0) * 0.03;
      group.rotation.z = Math.sin(qt * 2.0 + 1) * 0.012;
      head.rotation.x = 0;
      head.rotation.z = Math.sin(qt * 0.7) * 0.03;
      earL.rotation.y = Math.sin(qt * 0.9) * 0.12;
      earR.rotation.y = -Math.sin(qt * 0.9 + 0.4) * 0.12;
      armL.rotation.z = -ARM_REST + Math.sin(qt * 2.0 + 0.5) * 0.02;

      // wave flourish
      if (waveStart < 0 && now > nextWave) waveStart = now;
      if (waveStart >= 0) {
        const p = (now - waveStart) / WAVE_MS;
        if (p >= 1) {
          waveStart = -1;
          nextWave = now + 20000 + Math.random() * 20000;
          armR.rotation.z = ARM_REST;
        } else {
          // raise (0–0.25), wiggle (0.25–0.75), lower (0.75–1) — stepped
          const raise = p < 0.25 ? p / 0.25 : p > 0.75 ? (1 - p) / 0.25 : 1;
          const qRaise = Math.floor(raise * 6) / 6;
          const wiggle = p >= 0.25 && p <= 0.75 ? Math.sin(qt * 14) * 0.25 : 0;
          armR.rotation.z = ARM_REST + qRaise * 2.1 + wiggle;
        }
      } else {
        armR.rotation.z = ARM_REST + Math.sin(qt * 2.0) * 0.02;
      }
    } else {
      // away: slump — head down, ears drooped, arms dangling, slow breathing
      group.position.y = Math.sin(t * 0.8) * 0.012 - 0.06;
      group.rotation.z = 0;
      head.rotation.x = 0.38;
      head.rotation.z = 0.06;
      earL.rotation.y = 0.45;
      earR.rotation.y = -0.45;
      armL.rotation.z = -0.32;
      armR.rotation.z = 0.32;
    }

    // blink
    if (now > nextBlink) {
      blinkUntil = now + 130;
      nextBlink = now + 1800 + Math.random() * 3200;
    }
    const blinking = now < blinkUntil || mode === "away";
    eyes.forEach((e) => e.scale.setY(blinking ? 0.12 : 1));

    renderer.render(scene, camera);
  }
  requestAnimationFrame(animate);

  return {
    setMode(m) {
      mode = m;
    },
    dispose() {
      disposed = true;
      renderer.dispose();
    },
  };
}
```

- [ ] **Step 2: Verify**

Run: `npm run check && npm run build` — expected green. (`eyeL`/`eyeR` are declared inside `buildRat` — confirm no scope errors from the destructure; svelte-check covers the TS.)

- [ ] **Step 3: Visual smoke-test in a plain browser (no Tauri needed)**

```bash
npm run dev -- --port 5180 &
sleep 3 && curl -s http://localhost:5180/avatar.html | grep -q app && echo PAGE_OK
```

Open `http://localhost:5180/avatar.html` if a browser tool is available; otherwise rely on the live Tauri verification in Task 8. Kill the dev server after (`kill %1`).

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(shell): biped torso-up rat with hands — wave flourish, slump away mode"
```

---

### Task 7: Window size, flush-bottom positioning, icon

**Files:**
- Modify: `apps/shell/src-tauri/tauri.conf.json:17-18` (avatar window size)
- Modify: `apps/shell/src-tauri/src/main.rs:51-63` (positioning block)
- Modify: `apps/shell/src-tauri/icons/icon.png` (regenerate)

- [ ] **Step 1: tauri.conf.json — avatar window entry: `"width": 320, "height": 380` → `"width": 180, "height": 240`**

- [ ] **Step 2: main.rs — replace the positioning block:**

```rust
            // avatar: bottom-left, flush with the screen bottom (the rat is a
            // torso-up bust; the screen edge is his crop line — spec 2026-06-11)
            if let Some(avatar) = app.get_webview_window("avatar") {
                if let Ok(Some(monitor)) = avatar.primary_monitor() {
                    let size = monitor.size();
                    let outer = avatar.outer_size().unwrap_or(tauri::PhysicalSize {
                        width: 180,
                        height: 240,
                    });
                    let x = monitor.position().x + 16;
                    let y = monitor.position().y + size.height as i32 - outer.height as i32;
                    let _ = avatar.set_position(PhysicalPosition { x, y });
                }
```

(The trailing dashboard close-to-hide handler stays exactly as is.)

- [ ] **Step 3: Regenerate the icon (THUG2 sticker style):**

```bash
cd ~/rato/apps/shell/src-tauri && python3 - <<'EOF'
from PIL import Image, ImageDraw

S = 128
img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(img)

PAPER = (247, 241, 226, 255)   # #f7f1e2
INK = (30, 26, 21, 255)        # #1e1a15
ORANGE = (242, 107, 29, 255)   # #f26b1d

# hard offset shadow, then die-cut sticker card
d.rounded_rectangle([10, 10, S - 4, S - 4], radius=18, fill=INK)
d.rounded_rectangle([4, 4, S - 10, S - 10], radius=18, fill=PAPER, outline=INK, width=3)

# ink rat glyph: ears + head triangle, orange nose
d.ellipse([24, 22, 58, 56], outline=INK, width=6)
d.ellipse([68, 22, 102, 56], outline=INK, width=6)
d.polygon([(30, 46), (96, 46), (63, 102)], fill=INK)
d.ellipse([47, 54, 56, 63], fill=PAPER)
d.ellipse([70, 54, 79, 63], fill=PAPER)
d.ellipse([57, 88, 69, 100], fill=ORANGE)

img.save("icons/icon.png")
print("written", img.size)
EOF
```

- [ ] **Step 4: Rebuild the shell binary**

```bash
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"
cargo build --release --manifest-path ~/rato/apps/shell/src-tauri/Cargo.toml
```

Expected: `Finished release` (~1 min incremental).

- [ ] **Step 5: Commit**

```bash
cd ~/rato && git add -A && git commit -m "feat(shell): 180x240 avatar flush with screen bottom + THUG2 icon"
```

---

### Task 8: Live verification + tag

**Files:** none (operational)

- [ ] **Step 1: Relaunch on the live session**

```bash
pkill -x rato-shell; sleep 1
cd ~/rato && nohup env DISPLAY=:0 RAT_LOG=info ./apps/shell/src-tauri/target/release/rato-shell >/tmp/rato-shell.log 2>&1 &
sleep 3 && pgrep -x rato-shell && echo RUNNING
```

- [ ] **Step 2: Programmatic checks**

```bash
DISPLAY=:0 xwininfo -root -tree | grep '"rato"'   # expect 180x240 at x=16, y=screenH-240
DISPLAY=:0 xprop -id $(DISPLAY=:0 xwininfo -root -tree | grep '"rato"' | grep -o '0x[0-9a-f]*' | head -1) _NET_WM_STATE | grep ABOVE
```

- [ ] **Step 3: Operator checklist (needs eyes)** — ask the operator to confirm:
  - Rat reads as a front-facing biped bust with hands, rising from the screen bottom
  - Idle: breathing/sway/blink; occasional hand wave
  - Dashboard + chips show the paper/sticker/tape look, Anton/Marker fonts render (not fallbacks)
  - `systemctl --user stop ratd` flips NET chip red; `start` flips it back
  - Away mode (15 min idle, or shorten via daemon config if impatient) slumps the rat

- [ ] **Step 4: Tag**

```bash
cd ~/rato && git tag redesign-thug2 && echo tagged
```

(The `m2-shell` tag decision: tag M2 only after this redesign passes operator acceptance, since M2 acceptance was reopened by this feedback — tag both `m2-shell` and `redesign-thug2` at this commit.)

---

## Self-Review (done at write time)

- **Spec coverage:** §1 character → Task 6; §2 window/positioning → Tasks 5+7; §3 tokens/fonts/components/dashboard/avatar overlays → Tasks 1–5; §4 icon → Task 7; §6 acceptance → Task 8. No gaps.
- **Placeholders:** none — all code complete.
- **Type consistency:** `RatParts` fields match destructure in `mountRat`; `Rat3D` interface unchanged so `Avatar.svelte` needs no script edits; canvas 150×180 matches `RES_W/RES_H` and the 180×216 CSS box; token names in components exist in tokens.css (`--hud-ink`, `--hud-ink-dim`, `--hud-tape`, `--hud-font-marker`, `--hud-font-head`, `--hud-font-data`, `--hud-shadow-sm`, `.hud-grunge`, `.hud-tape`, `.hud-marker-stroke`).
