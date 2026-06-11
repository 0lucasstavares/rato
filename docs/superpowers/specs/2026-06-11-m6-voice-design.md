# M6 — Voice (wake-word / VAD / STT / intents / TTS / voice-approval) design

**Date:** 2026-06-11
**Status:** approved (autonomous-goal mode; decisions from ARCHITECTURE.md §15, §11, §10, §18-M6, §19)
**Acceptance (§18):** both languages wake & command reliably; pre-wake audio provably never persisted
(code audit + fs-watch test); slug-gated voice approval works.

## Reality constraints (binding)

Operator absent (no `sudo`, no interactive mic consent, no GPU assumptions, no model downloads this
session). The full stack — PipeWire mic, openWakeWord ONNX, Silero VAD, whisper-rs, Piper TTS — is
hardware/model/asset bound. M6 lands the **deterministic core** behind seams (audio source, wake
detector, VAD, STT, TTS, intent router) with real backends optional-dep + feature-gated and degrading
to `unavailable` when their platform/assets are missing. Fakes drive every test. The pre-wake
non-persistence guarantee is enforced **structurally** (RAM-only ring, no path to disk) and proven by
a code audit + an fs-watch test, not by trusting runtime behavior.

## Decisions (autonomous defaults)

| Question | Decision | Why |
|---|---|---|
| Audio capture | `AudioSource` trait. Real `PipeWireMic` (16 kHz mono) behind `--features mic`; `FakeAudioSource` (scripted PCM) for tests. Absent feature/runtime → `unavailable`, mic sensor sits `unavailable` in SensorGate. | mic stack can't run headless/without consent. |
| Pre-wake ring | 8 s RAM-only ring (`Vec`/`VecDeque` in an `mlock`ed, `zeroize`-on-drop buffer), continuously overwritten; **no method writes it to disk, transcribes, or embeds it**. Distinct from the §5 on-disk ambient ring (that path only runs with the explicit ambient-capture toggle, default ring-only/no-transcribe). | §15 verbatim; structural guarantee is the whole point — make it impossible, not policy-gated. |
| Wake word | `WakeDetector` trait. Real `OpenWakeWord` (4 ONNX: "rat"/"hey rat"/"rato"/"ei rato") behind `--features wake` (optional `ort`/onnx dep + bundled `.onnx` assets under `assets/wake/`); `FakeWakeDetector` (fires on a scripted frame index) for tests. Per-model threshold config. | model assets must be trainable/shippable separately; trait keeps the router testable now. |
| VAD + STT | `Vad` trait (real Silero behind `--features wake`; fake = fixed endpoints) + `SttEngine` trait (real `whisper-rs` small/large-v3-turbo behind `--features stt`, model path from config, lang constrained to {en,pt}; `FakeStt` returns scripted text+lang). Absent → `unavailable`. | whisper model files are large, not bundled; trait + fake for tests. |
| TTS | `TtsEngine` trait. Real `Piper` behind `--features tts` (en_US-lessac-medium / pt_BR-faber-medium voice files from config, per-mode length-scale/pitch from §13 personality); `FakeTts` (records the text it was asked to speak). | voice files are assets; render path testable via fake. |
| Intent router | Pure-logic `IntentRouter`: per-language local grammar (regex/keyword) for pause/resume sensors, private on/off, open dashboard, "pin that"/"pina isso" (pin last 2 min), snooze, mode switch, approval decisions; non-match → fallback chat intent (orchestrator → reply → TTS). Fully unit-testable with no audio. | the routable surface is the deterministic, high-value core; build it solid. |
| Voice-approval gate | voice may decide an approval ONLY if (a) a pending approval popup is currently visible, (b) its risk ≤ R2, (c) the utterance contains the approval's **two-word slug** rendered on the card. R3 never voice-approvable. Record `decided_via='voice'` + utterance id. Reuse the M4 `approvals.decide` path with a voice source + slug check. | §15 verbatim; this is the security-critical bit and must be exact. |
| Two-word slug | extend approvals: alongside the existing last-6-char typed slug (R3), generate a **two-word** spoken slug (adjective-animal from a fixed wordlist, derived deterministically from the approval id) rendered on every card; voice gate matches against it. Store/derive, don't add schema if derivable. | §15 needs a speakable slug distinct from the typed one. |
| Language policy | `voice_utterances.lang` from STT detection; typed chat via `lingua-rs` (en/pt). Most recent utterance/message sets `last_language`; popups/TTS render that side; `message_en`+`message_pt` always both generated (already true since M3). | §15 verbatim. |
| `voice_utterances` table | store migration **v6**: `voice_utterances(id PK, ts, lang, text, intent NULL, wake_word, handled INT)` per §10. Repo insert + recent. | §10 schema row, not yet created. |

## Components

1. **store migration v6**: `voice_utterances` table + repo.
2. **`rat-voice` crate**: traits `AudioSource`/`WakeDetector`/`Vad`/`SttEngine`/`TtsEngine` + Fake impls;
   RAM pre-wake ring (mlock/zeroize, no-disk invariant); `IntentRouter` (pure logic, both grammars);
   two-word slug derivation. All unit-tested with fakes.
3. **`rat-voice` real backends (feature-gated, best-effort)**: `PipeWireMic` (`mic`), `OpenWakeWord`+Silero
   (`wake`), `WhisperStt` (`stt`), `Piper` (`tts`). Default build pulls none of these.
4. **daemon wiring**: voice loop (only when mic enabled + features present): pre-wake ring → wake → VAD
   endpoint → STT → record `voice_utterances` → IntentRouter → execute local intent OR fallback chat;
   TTS reply via DialogueBox. Voice-approval gate integrated with `approvals.decide` (source=voice).
5. **CLI**: `rat voice status` (backend availability), `rat voice say "<text>"` (TTS smoke when `tts`),
   `rat utterances` (recent). RPC additive: `voice.status`, `voice.utterances`.
6. **Shell**: avatar ear-perk + MIC chip pulse on wake; DialogueBox renders STT/replies in `last_language`;
   Settings voice/wake toggles + per-language test buttons.

## Testing (§19)

Deterministic (no audio hardware): IntentRouter grammar tables (en + pt, every intent + the non-match
fallback); voice-approval gate (visible+R2+correct slug → decides; missing popup / R3 / wrong slug →
refused) — security-critical, exhaustive; two-word slug determinism; STT lang→`last_language` plumbing
via FakeStt; **pre-wake non-persistence: an fs-watch test that runs the wake/VAD path against
FakeAudioSource while watching the data/state dirs and asserts NOTHING is written from the pre-wake
ring** + a grep/code-audit assertion that the pre-wake buffer type has no serialize/write path; migration
v5→v6. Operator live-smoke (features on, mic consent): both languages wake ≤ threshold, command executes,
false-accept rate eyeballed; Piper speaks both voices.

## Out of scope (deferred)

openWakeWord model training (offline asset pipeline; ship a documented script + placeholder assets),
GPU whisper tuning, ambient continuous transcription (stays default-off), soak-measured false-accept
tuning (M8), barge-in/duplex conversation.
