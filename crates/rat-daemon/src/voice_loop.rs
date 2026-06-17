use anyhow::Result;
use rat_store::rows::NewVoiceUtterance;
use rat_store::store::Store;
use rat_voice::intent::Intent;
use rat_voice::traits::{AudioSource, BackendHealth, SttEngine, TtsEngine, Vad, WakeDetector};
use rat_voice::{IntentRouter, Lang};

use crate::pins::{PinKind, PinService};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceTickOutcome {
    pub utterance_id: String,
    pub intent: Intent,
    pub handled: bool,
}

pub struct VoiceLoop<A, W, V, S, T> {
    pub audio: A,
    pub wake: W,
    pub vad: V,
    pub stt: S,
    pub tts: T,
    pub router: IntentRouter,
}

impl<A, W, V, S, T> VoiceLoop<A, W, V, S, T>
where
    A: AudioSource,
    W: WakeDetector,
    V: Vad,
    S: SttEngine,
    T: TtsEngine,
{
    pub fn health(&self) -> Vec<(&'static str, BackendHealth)> {
        vec![
            ("mic", self.audio.health()),
            ("wake", self.wake.health()),
            ("vad", self.vad.health()),
            ("stt", self.stt.health()),
            ("tts", self.tts.health()),
        ]
    }

    pub async fn tick(
        &mut self,
        store: &Store,
        visible_approval_id: Option<&str>,
        pin_service: Option<&PinService>,
    ) -> Result<Option<VoiceTickOutcome>> {
        if self
            .health()
            .iter()
            .any(|(_, health)| !matches!(health, BackendHealth::Ok))
        {
            return Ok(None);
        }

        let Some(frame) = self.audio.next_frame()? else {
            return Ok(None);
        };
        let Some(wake) = self.wake.detect(&frame)? else {
            return Ok(None);
        };
        if !self.vad.is_speech(&frame)? {
            return Ok(None);
        }

        let transcript = self.stt.transcribe(&frame)?;
        let intent = self.router.route(transcript.lang, &transcript.text);
        let handled = is_local_intent(&intent);
        let intent_name = intent_name(&intent).to_string();

        let utterance = store
            .insert_voice_utterance(NewVoiceUtterance {
                lang: lang_code(transcript.lang).to_string(),
                text: transcript.text.clone(),
                intent: Some(intent_name),
                wake_word: wake.word,
                handled,
            })
            .await?;

        apply_voice_intent(
            store,
            &intent,
            visible_approval_id,
            pin_service,
            &utterance.id,
            utterance.ts,
        )
        .await?;

        Ok(Some(VoiceTickOutcome {
            utterance_id: utterance.id,
            intent,
            handled,
        }))
    }
}

async fn apply_voice_intent(
    store: &Store,
    intent: &Intent,
    visible_approval_id: Option<&str>,
    pin_service: Option<&PinService>,
    utterance_id: &str,
    decided_at: i64,
) -> Result<()> {
    match intent {
        Intent::ApprovalApprove { slug } | Intent::ApprovalDeny { slug } => {
            let Some(approval_id) = visible_approval_id else {
                return Ok(());
            };
            let Some(approval) = store.get_approval(approval_id.to_string()).await? else {
                return Ok(());
            };
            if approval.status != "pending" || approval.risk >= 3 {
                return Ok(());
            }
            if rat_voice::spoken_slug(&approval.id) != *slug {
                return Ok(());
            }
            let status = match intent {
                Intent::ApprovalApprove { .. } => "approved",
                Intent::ApprovalDeny { .. } => "denied",
                _ => unreachable!(),
            };
            store
                .decide_approval(
                    approval.id,
                    status.to_string(),
                    decided_at,
                    "voice".to_string(),
                    Some(format!("utterance_id={utterance_id}")),
                )
                .await?;
        }
        Intent::PinRecent { minutes } => {
            let Some(service) = pin_service else {
                return Ok(());
            };
            if let Err(e) = service
                .pin_recent(
                    rat_ring::Media::Screen,
                    *minutes,
                    PinKind::Manual,
                    format!("voice pin_recent utterance_id={utterance_id}"),
                )
                .await
            {
                tracing::warn!("voice pin_recent failed: {e}");
            }
        }
        _ => {}
    }
    Ok(())
}

fn is_local_intent(intent: &Intent) -> bool {
    !matches!(intent, Intent::Chat { .. })
}

fn intent_name(intent: &Intent) -> &'static str {
    match intent {
        Intent::PauseSensors => "pause_sensors",
        Intent::ResumeSensors => "resume_sensors",
        Intent::PrivateOn => "private_on",
        Intent::PrivateOff => "private_off",
        Intent::OpenDashboard => "open_dashboard",
        Intent::PinRecent { .. } => "pin_recent",
        Intent::Snooze => "snooze",
        Intent::ModeSwitch { .. } => "mode_switch",
        Intent::ApprovalApprove { .. } => "approval_approve",
        Intent::ApprovalDeny { .. } => "approval_deny",
        Intent::Chat { .. } => "chat",
    }
}

fn lang_code(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "en",
        Lang::Pt => "pt",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rat_core::clock::Clock;
    use rat_core::clock::FakeClock;
    use rat_ring::{Media, RingKey, RingWriter};
    use rat_store::rows::NewApproval;
    use rat_voice::traits::{FakeAudioSource, FakeStt, FakeTts, FakeVad, FakeWakeDetector};
    use rat_voice::{spoken_slug, Intent};
    use tempfile::tempdir;

    use super::*;

    struct StaticKeyStore([u8; 32]);

    impl crate::pins::PinKeyStore for StaticKeyStore {
        fn load_or_create(&self) -> anyhow::Result<[u8; 32]> {
            Ok(self.0)
        }
    }

    async fn store_at(now_ms: i64) -> Store {
        let tmp = tempdir().unwrap();
        let db = tmp.path().join("t.db");
        let clock: Arc<dyn Clock> = FakeClock::at(now_ms);
        let store = Store::open(&db, clock).unwrap();
        std::mem::forget(tmp);
        store
    }

    #[tokio::test]
    async fn fake_voice_loop_records_utterance_and_routes_local_intent() {
        let store = store_at(10_000).await;
        let mut loop_ = VoiceLoop {
            audio: FakeAudioSource::new(vec![vec![0.1, 0.2]]),
            wake: FakeWakeDetector::fire_on(0, "hey rat"),
            vad: FakeVad::new(true),
            stt: FakeStt::new("open dashboard", Lang::En),
            tts: FakeTts::default(),
            router: IntentRouter,
        };

        let outcome = loop_.tick(&store, None, None).await.unwrap().unwrap();
        assert_eq!(outcome.intent, Intent::OpenDashboard);
        assert!(outcome.handled);

        let utterances = store.recent_voice_utterances(10).await.unwrap();
        assert_eq!(utterances.len(), 1);
        assert_eq!(utterances[0].id, outcome.utterance_id);
        assert_eq!(utterances[0].wake_word, "hey rat");
        assert_eq!(utterances[0].intent.as_deref(), Some("open_dashboard"));
        assert!(utterances[0].handled);
    }

    #[tokio::test]
    async fn voice_approval_records_voice_channel_and_utterance_id() {
        let store = store_at(20_000).await;
        let approval = store
            .insert_approval(NewApproval {
                kind: "test".into(),
                risk: 2,
                title: "Allow thing".into(),
                reason: "test".into(),
                cwd: None,
                target: None,
                agent_identity: "test".into(),
                payload: serde_json::json!({}),
                expected_impact: serde_json::json!({}),
                expires_at: 30_000,
            })
            .await
            .unwrap();
        let slug = spoken_slug(&approval.id);
        let mut loop_ = VoiceLoop {
            audio: FakeAudioSource::new(vec![vec![0.1, 0.2]]),
            wake: FakeWakeDetector::fire_on(0, "hey rat"),
            vad: FakeVad::new(true),
            stt: FakeStt::new(format!("approve {slug}"), Lang::En),
            tts: FakeTts::default(),
            router: IntentRouter,
        };

        let outcome = loop_
            .tick(&store, Some(&approval.id), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(outcome.intent, Intent::ApprovalApprove { slug });

        let decided = store.get_approval(approval.id).await.unwrap().unwrap();
        assert_eq!(decided.status, "approved");
        assert_eq!(decided.decided_via.as_deref(), Some("voice"));
        let expected_note = format!("utterance_id={}", outcome.utterance_id);
        assert_eq!(
            decided.decision_note.as_deref(),
            Some(expected_note.as_str())
        );
    }

    #[tokio::test]
    async fn voice_pin_recent_creates_manual_screen_pin_when_ring_has_segment() {
        let tmp = tempdir().unwrap();
        let clock = FakeClock::at(30_000);
        let store = Store::open(&tmp.path().join("t.db"), clock.clone()).unwrap();
        let ring = Arc::new(RingWriter {
            dir: tmp.path().join("ring"),
            segment_secs: 10,
            ttl_secs: 1_200,
            clock: clock.clone(),
        });
        let ring_key = Arc::new(RingKey::ephemeral());
        ring.write_segment(Media::Screen, b"screen bytes", &ring_key)
            .unwrap();
        let pins = PinService::new(
            store.clone(),
            ring.clone(),
            ring_key,
            Arc::new(StaticKeyStore([9u8; 32])),
            tmp.path().join("pins"),
            clock.clone(),
        );
        let mut loop_ = VoiceLoop {
            audio: FakeAudioSource::new(vec![vec![0.1, 0.2]]),
            wake: FakeWakeDetector::fire_on(0, "hey rat"),
            vad: FakeVad::new(true),
            stt: FakeStt::new("pin that", Lang::En),
            tts: FakeTts::default(),
            router: IntentRouter,
        };

        let outcome = loop_
            .tick(&store, None, Some(&pins))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(outcome.intent, Intent::PinRecent { minutes: 2 });
        assert!(outcome.handled);

        let pin_rows = store.list_pins().await.unwrap();
        assert_eq!(pin_rows.len(), 1);
        assert_eq!(pin_rows[0].kind, "manual");
        assert_eq!(pin_rows[0].media, "screen");
        assert!(pin_rows[0].reason.contains(&outcome.utterance_id));
        assert!(std::path::Path::new(&pin_rows[0].path).exists());
    }
}
