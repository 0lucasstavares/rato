use rat_proto::{NewObservation, Observation};
use rat_ring::{Media, RingKey, RingWriter, Segment};
use rat_store::rows::Pin;
use rat_store::store::Store;
use rat_vision::autopin::autopin_reason;
use rat_vision::ocr::OcrEngine;
use rat_vision::pipeline::CapturePipeline;
use rat_vision::screen::{ScreenSource, SourceHealth};
use serde_json::json;

use crate::pins::{PinKind, PinService};

#[derive(Debug, Clone)]
pub struct CaptureTickResult {
    pub segment: Segment,
    pub observation: Option<Observation>,
    pub auto_pin: Option<Pin>,
}

pub async fn run_capture_tick<S, O>(
    pipeline: &mut CapturePipeline<S, O>,
    ring: &RingWriter,
    ring_key: &RingKey,
    store: &Store,
    pin_service: Option<&PinService>,
) -> anyhow::Result<Option<CaptureTickResult>>
where
    S: ScreenSource,
    O: OcrEngine,
{
    if !matches!(pipeline.source.health(), SourceHealth::Ok) {
        return Ok(None);
    }

    let Some(output) = pipeline.tick() else {
        return Ok(None);
    };

    let segment = ring.write_segment(Media::Screen, &output.frame_jpeg, ring_key)?;
    let mut observation = None;
    let mut auto_pin = None;

    if !output.ocr_delta.trim().is_empty() {
        let obs = store
            .add_observation(NewObservation {
                kind: "ocr".to_string(),
                content: output.ocr_delta.clone(),
                meta: json!({
                    "window_title": output.window_title,
                    "captured_ms": output.captured_ms,
                }),
                ..Default::default()
            })
            .await?;

        if let (Some(service), Some(reason)) = (pin_service, autopin_reason(&output.ocr_delta)) {
            match service
                .pin_recent(Media::Screen, 1, PinKind::Auto, reason.clone())
                .await
            {
                Ok(pin) => auto_pin = Some(pin),
                Err(e) => tracing::warn!("auto-pin failed ({reason}): {e}"),
            }
        }

        observation = Some(obs);
    }

    Ok(Some(CaptureTickResult {
        segment,
        observation,
        auto_pin,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    use rat_core::clock::FakeClock;
    use rat_vision::ocr::{FakeOcr, OcrBlock};
    use rat_vision::screen::{FakeScreenSource, Frame, ScreenResult};

    fn frame(captured_ms: i64, r: u8, g: u8, b: u8) -> Frame {
        let rgba = (0..64 * 64).flat_map(|_| [r, g, b, 255]).collect();
        Frame {
            rgba,
            width: 64,
            height: 64,
            window_title: Some("tests".to_string()),
            captured_ms,
        }
    }

    fn block(text: &str) -> OcrBlock {
        OcrBlock {
            text: text.to_string(),
            bbox: (0, 0, 100, 20),
        }
    }

    #[tokio::test]
    async fn capture_tick_writes_ring_and_searchable_ocr_observation() {
        let tmp = tempfile::tempdir().unwrap();
        let clock = FakeClock::at(10_000);
        let store = Store::open(&tmp.path().join("rato.db"), clock.clone()).unwrap();
        let ring = RingWriter {
            dir: tmp.path().join("ring"),
            segment_secs: 10,
            ttl_secs: 1_200,
            clock: clock.clone(),
        };
        let ring_key = RingKey::ephemeral();
        let source = FakeScreenSource::new(vec![ScreenResult::Frame(frame(10_000, 10, 20, 30))]);
        let ocr = FakeOcr::new(vec![vec![block("panicked at src/main.rs:12")]]);
        let mut pipeline = CapturePipeline::new(source, ocr);

        let result = run_capture_tick(&mut pipeline, &ring, &ring_key, &store, None)
            .await
            .unwrap()
            .expect("unique frame should produce output");

        assert_eq!(result.segment.media, Media::Screen);
        let observation = result
            .observation
            .expect("non-empty OCR delta inserts observation");
        assert_eq!(observation.kind, "ocr");
        assert!(observation.content.contains("panicked at"));

        let hits = store
            .fts_observations("panicked".to_string(), 5)
            .await
            .unwrap();
        assert_eq!(hits, vec![observation.id]);
    }
}
