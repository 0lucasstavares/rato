# Avatar + HUD redesign — biped rat, THUG2 visual language

**Date:** 2026-06-11
**Status:** approved
**Context:** M2 shell acceptance feedback from the operator. The procedural rat read as an
oversized side-profile quadruped floating above the screen bottom, and the HUD-PS2 theme read
as "mainframe terminal". Reference image provided: low-poly upright rat, front-facing,
GMod/PS2 era. Visual direction requested: Tony Hawk's Underground 2 menus (skate-zine
collage), applied everywhere.

Supersedes the visual-language half of ARCHITECTURE.md §13 (HUD-PS2 palette/typography);
the §13 component inventory (panel/chip/tab/meter) and the low-poly/12fps avatar principles
stand. The M7 Blender glTF milestone is unaffected — the procedural rat remains a placeholder.

## 1. Character (rat3d.ts rebuild)

A bipedal rat, framed **torso-up**: the window's bottom edge crops him at the waist, so he
rises out of the screen bottom like a fighting-game select bust. Head-on toward the viewer,
filling ~85% of canvas height. No feet or tail (cropped out).

Anatomy, all flat-shaded low-poly primitives (icosahedron/cone/cylinder, detail 0):

- **Torso:** plump grey pear — wide at the crop line, narrower at the chest/shoulders.
- **Head:** round sphere directly on the shoulders (no neck); short blunt snout cone pointing
  at the camera; pink nose tip.
- **Ears:** big circular discs, pink inner / grey rim, angled slightly outward. The
  identity-carrying feature.
- **Eyes:** black beads, front-facing.
- **Arms + hands:** proper biped arms relaxed at his sides, ending in simple low-poly mitt
  hands (palm + thumb wedge). Not tucked rodent paws.

Fur `#8d93a1` / dark `#6c7280`, pink `#d99aa8`, eye `#16181d` — unchanged.

**Animation** (12fps-quantized idle, as today):
- Active: breathing bob, slight side sway, ear twitches, blink (existing timing); rare idle
  flourish every ~20–40s — raises one hand (small wave) or chin-scratch.
- Away: slump — head drops forward, ears droop, arms dangle, slow breathing. Replaces the
  old "lie down" (a cropped bust can't lie down). `zzz` overlay stays.

Render: low-res offscreen (~160px) upscaled nearest-neighbor (`image-rendering: pixelated`)
for the PS2 chunk. Camera head-on at chest height.

## 2. Window + positioning

- Avatar window: **180×240** (tauri.conf.json; matching fallback in main.rs).
- main.rs: `y = monitor_y + screen_height − window_height` (the old `− margin − 48` is
  removed); `x = monitor_x + 16`. Flush with the screen bottom so the crop line sits on the
  screen edge.
- Grip bar (drag handle + LED chips) becomes a slim strip at the very top of the window.
- Avatar.svelte layout: canvas anchored to window bottom, no dead space below.

## 3. THUG2 visual language (tokens.css rewrite + component restyle)

The HUD-PS2 dark-terminal theme is replaced wholesale. The dashboard flips to a **light**
paper theme. No image assets — all texture is CSS/SVG (inline `feTurbulence` data-URIs).

### Tokens (same custom-property names where a 1:1 replacement exists)

| Token | Value | Role |
|---|---|---|
| `--hud-bg` | `#e9e1cb` | aged cream paper |
| `--hud-panel` | `#f7f1e2` | paper card |
| `--hud-ink` | `#1e1a15` | text, borders (replaces `--hud-text`) |
| `--hud-ink-dim` | `#6b6256` | secondary text (replaces `--hud-text-dim`) |
| `--hud-accent` | `#f26b1d` | spray orange |
| `--hud-ok` | `#3fa544` | sticker green |
| `--hud-warn` | `#f2b705` | sticker yellow |
| `--hud-danger` | `#d23b2e` | sticker red |
| `--hud-info` | `#2a6fc8` | sticker blue |

Removed: bevel line tokens (`--hud-line*`), panel-raised, scanline/dither classes.
Hard shadows: `box-shadow: 4px 4px 0 var(--hud-ink)` (no blur anywhere).
Grunge: one shared low-opacity `feTurbulence` noise overlay class on `body` + cards.
Rotation: cards/stickers get alternating ±0.5–1° via `:nth-child` rules.

### Type (OFL fonts committed to `apps/shell/src/assets/fonts/`, woff2)

| Use | Font | Fallback stack |
|---|---|---|
| Headers/tabs | Anton | `"Arial Narrow", Impact, sans-serif` |
| Scrawl labels | Permanent Marker | `"Comic Sans MS", cursive` |
| Body | Barlow | `system-ui, sans-serif` |
| Data readouts | (keep) IBM Plex Mono stack | `monospace` |

### Components

- **HudPanel** → paper card: ink border 2px, hard shadow, tape strips (rotated
  semi-transparent pseudo-elements) on two corners, Permanent Marker title.
- **StatusChip** → die-cut sticker: 3px white border, hard shadow, Anton label, colored
  state dot, per-chip rotation. States ok/off/warn/err map to sticker colors.
- **TabBar** → Anton uppercase tabs; active tab gets a rough orange marker underline
  (skewed/offset border or SVG stroke).
- **MeterBar** → ink outline, orange marker fill.
- **hud-btn** → sticker button: paper card, 2px ink border, hard shadow; `:active`
  translates 3px and collapses the shadow; hover tilts ~1°.
- **Dashboard** → cream bg + grunge, big faint rotated "RATO" Anton stencil watermark in a
  corner. Tab/page structure unchanged.
- **Avatar overlays** → grip strip = "tape" band with mini sticker LEDs; quick panel =
  small paper card. The 3D rat is not themed.

## 4. App icon

Regenerate `src-tauri/icons/icon.png` in the new language: cream sticker, ink rat glyph,
orange accent, white die-cut border.

## 5. Out of scope

- Blender glTF model, personality menu, voice — M7.
- Theme switching/dark mode — single theme only.
- Dashboard information architecture — tabs and data unchanged.

## 6. Acceptance

1. Avatar window 180×240, flush with screen bottom-left (16px left margin, 0 bottom).
2. Rat reads as a front-facing biped bust with hands; idle + away + blink + flourish work.
3. Dashboard and avatar overlays show the paper/sticker/tape language; no navy, no
   scanlines; fonts load (with working fallbacks).
4. `npm run build` + `npm run check` green; shell rebuild green; live verification on the
   operator's session (operator confirms the look).
