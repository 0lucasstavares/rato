use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context};
use rand::RngCore;
use rat_core::clock::Clock;
use rat_core::id::new_id;
use rat_proto::RingMediaStatusDto;
use rat_ring::{seal, Media, RingKey, RingWriter};
use rat_store::rows::{NewPin, Pin};
use rat_store::store::Store;
use serde_json::json;

const PIN_KEY_ACCOUNT: &str = "pin-key";
const AUTO_PIN_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinKind {
    Auto,
    Manual,
}

impl PinKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PinKind::Auto => "auto",
            PinKind::Manual => "manual",
        }
    }
}

pub trait PinKeyStore: Send + Sync {
    fn load_or_create(&self) -> anyhow::Result<[u8; 32]>;
}

#[derive(Debug, Default)]
pub struct KeyringPinKeyStore;

impl PinKeyStore for KeyringPinKeyStore {
    fn load_or_create(&self) -> anyhow::Result<[u8; 32]> {
        if let Ok(from_env) = std::env::var("RATO_PIN_KEY") {
            if !from_env.trim().is_empty() {
                return decode_hex_32(from_env.trim()).context("invalid RATO_PIN_KEY");
            }
        }

        match rat_brain::keys::get_secret(PIN_KEY_ACCOUNT) {
            Ok(secret) => decode_hex_32(secret.trim()).context("invalid stored rato/pin-key"),
            Err(_) => {
                let mut bytes = [0u8; 32];
                rand::rngs::OsRng.fill_bytes(&mut bytes);
                rat_brain::keys::set_secret(PIN_KEY_ACCOUNT, &encode_hex(&bytes))
                    .context("storing rato/pin-key")?;
                Ok(bytes)
            }
        }
    }
}

#[derive(Clone)]
pub struct PinService {
    store: Store,
    ring: Arc<RingWriter>,
    ring_key: Arc<RingKey>,
    key_store: Arc<dyn PinKeyStore>,
    pins_dir: PathBuf,
    clock: Arc<dyn Clock>,
}

impl PinService {
    pub fn new(
        store: Store,
        ring: Arc<RingWriter>,
        ring_key: Arc<RingKey>,
        key_store: Arc<dyn PinKeyStore>,
        pins_dir: PathBuf,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            store,
            ring,
            ring_key,
            key_store,
            pins_dir,
            clock,
        }
    }

    pub async fn pin_recent(
        &self,
        media: Media,
        minutes: u32,
        kind: PinKind,
        reason: impl Into<String>,
    ) -> anyhow::Result<Pin> {
        let minutes = minutes.clamp(1, 24 * 60);
        let now = self.clock.now_ms();
        let cutoff_ms = now - i64::from(minutes) * 60_000;
        let segments = self
            .ring
            .list_segments(media)
            .with_context(|| format!("listing {} ring segments", media.as_str()))?
            .into_iter()
            .filter(|s| s.created_ms >= cutoff_ms)
            .collect::<Vec<_>>();

        if segments.is_empty() {
            anyhow::bail!(
                "no {} ring segments in the last {} minute(s)",
                media.as_str(),
                minutes
            );
        }

        let pin_id = new_id();
        let pin_dir = self.pins_dir.join(&pin_id);
        rat_core::paths::ensure_private_dir(&pin_dir)
            .with_context(|| format!("creating pin dir {}", pin_dir.display()))?;

        let pin_key = RingKey::from_bytes(self.key_store.load_or_create()?);
        for seg in &segments {
            let plain = self
                .ring
                .read_segment(seg, &self.ring_key)
                .with_context(|| format!("reading ring segment {}", seg.path.display()))?;
            let sealed = seal(&pin_key, &plain, media.as_str().as_bytes());
            let file_name = seg
                .path
                .file_name()
                .ok_or_else(|| anyhow!("segment path has no filename: {}", seg.path.display()))?;
            std::fs::write(pin_dir.join(file_name), sealed)
                .with_context(|| format!("writing pin segment into {}", pin_dir.display()))?;
        }

        let expires_at = match kind {
            PinKind::Auto => Some(now + AUTO_PIN_TTL_MS),
            PinKind::Manual => None,
        };
        self.store
            .insert_pin_with_id(
                pin_id,
                NewPin {
                    kind: kind.as_str().to_string(),
                    media: media.as_str().to_string(),
                    path: pin_dir.display().to_string(),
                    expires_at,
                    reason: reason.into(),
                    meta: json!({
                        "segment_count": segments.len(),
                        "from_ms": cutoff_ms,
                        "to_ms": now,
                    }),
                },
            )
            .await
            .context("inserting pin row")
    }

    pub async fn list(&self) -> anyhow::Result<Vec<Pin>> {
        self.store.list_pins().await.context("listing pins")
    }

    pub fn ring_status(&self) -> anyhow::Result<Vec<RingMediaStatusDto>> {
        [Media::Screen, Media::Audio, Media::Clipboard]
            .into_iter()
            .map(|media| {
                let segments = self
                    .ring
                    .list_segments(media)
                    .with_context(|| format!("listing {} ring segments", media.as_str()))?;
                Ok(RingMediaStatusDto {
                    media: media.as_str().to_string(),
                    segment_count: segments.len() as u32,
                    oldest_ms: segments.first().map(|s| s.created_ms),
                    newest_ms: segments.last().map(|s| s.created_ms),
                    ttl_secs: self.ring.ttl_secs,
                })
            })
            .collect()
    }

    pub async fn unpin(&self, id: &str) -> anyhow::Result<()> {
        let pin = self
            .store
            .get_pin(id.to_string())
            .await
            .context("loading pin")?
            .ok_or_else(|| anyhow!("pin not found: {id}"))?;
        self.store
            .delete_pin(id.to_string())
            .await
            .context("deleting pin row")?;
        remove_dir_if_under(&pin.path, &self.pins_dir)?;
        Ok(())
    }
}

pub fn media_from_str(s: &str) -> anyhow::Result<Media> {
    match s {
        "screen" => Ok(Media::Screen),
        "audio" => Ok(Media::Audio),
        "clipboard" => Ok(Media::Clipboard),
        other => anyhow::bail!("media must be screen|audio|clipboard, got {other}"),
    }
}

fn remove_dir_if_under(path: &str, root: &Path) -> anyhow::Result<()> {
    let path = PathBuf::from(path);
    if !path.starts_with(root) {
        anyhow::bail!(
            "refusing to remove pin path outside {}: {}",
            root.display(),
            path.display()
        );
    }
    match std::fs::remove_dir_all(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

fn encode_hex(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for b in bytes {
        use std::fmt::Write;
        write!(out, "{b:02x}").expect("write to String cannot fail");
    }
    out
}

fn decode_hex_32(s: &str) -> anyhow::Result<[u8; 32]> {
    if s.len() != 64 {
        anyhow::bail!("expected 64 hex chars, got {}", s.len());
    }
    let mut out = [0u8; 32];
    for (idx, chunk) in s.as_bytes().chunks_exact(2).enumerate() {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        out[idx] = (hi << 4) | lo;
    }
    Ok(out)
}

fn hex_nibble(b: u8) -> anyhow::Result<u8> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => anyhow::bail!("invalid hex character"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rat_core::clock::FakeClock;

    struct StaticKeyStore([u8; 32]);

    impl PinKeyStore for StaticKeyStore {
        fn load_or_create(&self) -> anyhow::Result<[u8; 32]> {
            Ok(self.0)
        }
    }

    #[tokio::test]
    async fn pin_recent_copies_recent_ring_segments_and_unpin_removes_them() {
        let tmp = tempfile::tempdir().unwrap();
        let clock = FakeClock::at(1_000_000);
        let store = Store::open(&tmp.path().join("rato.db"), clock.clone()).unwrap();
        let ring = Arc::new(RingWriter {
            dir: tmp.path().join("ring"),
            segment_secs: 10,
            ttl_secs: 1_200,
            clock: clock.clone(),
        });
        let ring_key = Arc::new(RingKey::ephemeral());

        ring.write_segment(Media::Screen, b"frame 1", &ring_key)
            .unwrap();
        clock.advance(30_000);
        ring.write_segment(Media::Screen, b"frame 2", &ring_key)
            .unwrap();

        let service = PinService::new(
            store,
            ring,
            ring_key,
            Arc::new(StaticKeyStore([7u8; 32])),
            tmp.path().join("pins"),
            clock,
        );

        let pin = service
            .pin_recent(Media::Screen, 5, PinKind::Manual, "manual")
            .await
            .unwrap();
        assert_eq!(pin.kind, "manual");
        assert_eq!(pin.media, "screen");
        assert!(Path::new(&pin.path).is_dir());
        assert_eq!(std::fs::read_dir(&pin.path).unwrap().count(), 2);

        service.unpin(&pin.id).await.unwrap();
        assert!(!Path::new(&pin.path).exists());
        assert!(service.list().await.unwrap().is_empty());
    }
}
