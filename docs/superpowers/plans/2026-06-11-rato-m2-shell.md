# RATO M2 — Shell Implementation Plan (avatar + dashboard skeleton)

> Executed inline by the plan author. Interface contracts and acceptance below are binding; full code lives in the commits.

**Goal:** M2 per `docs/ARCHITECTURE.md` §18 — Tauri v2 shell with (a) transparent always-on-top avatar window (placeholder low-poly Three.js rat, idle animation, sensor LEDs, drag, click/dbl-click/right-click menus) and (b) dashboard skeleton (Now / Sensors / Settings tabs) — all on the HUD-PS2 design system. Thin client: zero business logic, every byte of state comes from `ratd` over the existing NDJSON-RPC socket.

**Acceptance (from spec):** avatar always-on-top on GNOME Wayland; LEDs track daemon state live; dashboard tabs render real data (status, sessions, observations, mode); popup-free skeleton is fine for M2.

**Environment facts:** Node 24.16 user-level at `~/.local/bin`. Tauri system libs (webkit2gtk-4.1 etc.) require a one-time `sudo apt install` by the operator — the Rust `src-tauri` build is gated on it; everything else proceeds.

---

## Task 1: `rat-client` crate (shared UDS RPC client)

Move `rat-cli/src/client.rs` into new `crates/rat-client` (lib), re-export `Client`; rat-cli and `src-tauri` both depend on it. Add a `call_value` convenience and a reconnect-on-error wrapper `ManagedClient` (used by the shell: one persistent connection, reconnect + re-hello on IO error, surface "daemon down" as a typed state).
**Tests:** existing rat-cli suite keeps passing (client moved, behavior identical); ManagedClient reconnects after daemon restart (integration test with in-process server dropped and rebound).

## Task 2: Frontend scaffold (`apps/shell`)

- Vite + Svelte 5 (runes) + TypeScript, **two entry pages**: `avatar.html`, `dashboard.html` (one Tauri window each).
- `src/lib/rpc.ts`: `rpc<T>(method, params)` → Tauri `invoke("rpc_call", …)`; typed mirrors of proto types in `src/lib/types.ts`; `poll(fn, ms)` helper with visibility pause.
- HUD-PS2 design system `src/ui/hud/`: `tokens.css` (palette per spec §13: bg #0B0E14, panel #141A24, line #3A4860, text #C8D4E0, accent #7CFF6B, warn #FFB02E, danger #FF5C5C, info #5CC8FF; 2px square borders, bevel, Departure-style pixel headers via local fallback stack, monospace body), components `HudPanel.svelte`, `TitleBar.svelte`, `StatusChip.svelte`, `MeterBar.svelte`, `TabBar.svelte`, scanline/dither CSS overlays (pure CSS, no binary assets in M2).
- **Check:** `npm run build` (vite) + `npm run check` (svelte-check) green.

## Task 3: Avatar window UI

- `src/avatar/`: Three.js scene — procedural low-poly rat (boxes/cones, flat-shaded, vertex-lit, nearest upscale render at 240px then CSS upscale for the PS2 look), idle bob + tail sway + blink loop.
- LED strip above the rat: `SCR` `MIC` `CLP` `NET` chips — M2 truth table: NET = daemon RPC reachable; CLP = on (clipboard sensor ships in daemon); SCR/MIC = "off" (M5/M6), away mode tints the rat (sleep posture = lying flat + zzz text).
- Interactions: top grip bar = drag (window.startDragging); single click body = quick panel (Open dashboard / Sensors snapshot); double click = open dashboard window; right click = mini menu (placeholder personality list, disabled). Position persisted via daemon? M2: persisted to `localStorage`, restored by rust setup on launch.
- Status poll every 2 s (`status` + `mode.get`).

## Task 4: Dashboard window UI

- `TabBar`: **Now** (daemon status meters, open session card, last 10 observations mission-log), **Sensors** (mode/idle meter, per-sensor board: shell/proc/git/clipboard/idle with live event counts from `events.recent`), **Settings** (read-only: version, socket, db path, proto). 5 s polls.

## Task 5: `src-tauri` shell process

- Tauri v2 app, two windows from `tauri.conf.json`: `avatar` (320×360, transparent, no decorations, alwaysOnTop, skipTaskbar, resizable false), `dashboard` (1100×720, hidden at start, decorations on).
- Rust side: `ManagedClient` (rat-client) behind a tokio Mutex; commands: `rpc_call(method, params) -> Value`, `open_dashboard()`, `daemon_ok() -> bool`. Setup hook: position avatar bottom-left (primary monitor work area − margins), restore saved position if frontend sends one.
- **Gated on system libs:** `cargo build` of src-tauri requires webkit2gtk-4.1 — operator runs the apt install; until then Task 5 code is written but unbuilt.

## Task 6: Service + acceptance

- `packaging/systemd/rato-shell.service` (`PartOf=graphical-session.target`, `ExecStart=<repo>/target/release/rato-shell`); `rat install` gains `--with-shell` (writes both units) — optional if time, else manual unit documented in README.
- Acceptance: `npm run build` green; `cargo build` of shell green (post-apt); launch `rato-shell` on the live session — avatar visible bottom-left, always-on-top, LEDs live (NET green with daemon up, flips red when `systemctl --user stop ratd`), dashboard opens on double-click showing real sessions/observations; tag `m2-shell`.
