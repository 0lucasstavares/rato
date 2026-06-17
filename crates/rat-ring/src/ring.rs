use std::path::{Path, PathBuf};
use std::sync::Arc;

use rat_core::clock::Clock;
use rat_core::id::new_id;

use crate::crypto::{open, seal, RingError, RingKey};

// ── Media ────────────────────────────────────────────────────────────────────

/// The kind of data stored in a ring segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Media {
    Screen,
    Audio,
    Clipboard,
}

impl Media {
    /// Directory-name representation (used in paths and as AAD).
    pub fn as_str(self) -> &'static str {
        match self {
            Media::Screen => "screen",
            Media::Audio => "audio",
            Media::Clipboard => "clipboard",
        }
    }
}

// ── Segment ──────────────────────────────────────────────────────────────────

/// Metadata for one ring-buffer segment on disk.
#[derive(Debug, Clone)]
pub struct Segment {
    pub id: String,
    pub path: PathBuf,
    pub created_ms: i64,
    pub media: Media,
}

// ── RingWriter ───────────────────────────────────────────────────────────────

/// Writes, reads, lists, and prunes encrypted ring-buffer segments.
pub struct RingWriter {
    pub dir: PathBuf,
    pub segment_secs: u64,
    pub ttl_secs: u64,
    pub clock: Arc<dyn Clock>,
}

impl RingWriter {
    /// Seal `bytes` and write them to `<dir>/<media>/<created_ms>-<ulid>.seg`.
    pub fn write_segment(
        &self,
        media: Media,
        bytes: &[u8],
        key: &RingKey,
    ) -> Result<Segment, RingError> {
        let created_ms = self.clock.now_ms();
        let id = new_id();
        let media_dir = self.dir.join(media.as_str());
        std::fs::create_dir_all(&media_dir)?;

        let filename = format!("{}-{}.seg", created_ms, id);
        let path = media_dir.join(&filename);

        let sealed = seal(key, bytes, media.as_str().as_bytes());
        std::fs::write(&path, &sealed)?;

        Ok(Segment {
            id,
            path,
            created_ms,
            media,
        })
    }

    /// Delete segments whose `created_ms < now - ttl_secs*1000`.
    /// Returns the number of files deleted.
    pub fn prune(&self) -> Result<u32, RingError> {
        let cutoff_ms = self.clock.now_ms() - (self.ttl_secs as i64) * 1_000;
        let mut deleted = 0u32;

        for entry in read_dir_entries(&self.dir)? {
            // Each immediate subdirectory is a media dir.
            if entry.is_dir() {
                for seg_entry in read_dir_entries(&entry)? {
                    if let Some(created_ms) = parse_created_ms(&seg_entry) {
                        if created_ms < cutoff_ms && std::fs::remove_file(&seg_entry).is_ok() {
                            deleted += 1;
                        }
                    }
                }
            }
        }
        Ok(deleted)
    }

    /// List all segments for a given media type, sorted by `created_ms` ascending.
    pub fn list_segments(&self, media: Media) -> Result<Vec<Segment>, RingError> {
        let media_dir = self.dir.join(media.as_str());
        if !media_dir.exists() {
            return Ok(vec![]);
        }

        let mut segments: Vec<Segment> = read_dir_entries(&media_dir)?
            .into_iter()
            .filter_map(|path| segment_from_path(&path, media))
            .collect();

        segments.sort_by_key(|s| s.created_ms);
        Ok(segments)
    }

    /// Read a segment from disk and decrypt it.
    pub fn read_segment(&self, seg: &Segment, key: &RingKey) -> Result<Vec<u8>, RingError> {
        let sealed = std::fs::read(&seg.path)?;
        open(key, &sealed, seg.media.as_str().as_bytes())
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Read all immediate children of `dir` as `PathBuf`s.
fn read_dir_entries(dir: &Path) -> Result<Vec<PathBuf>, RingError> {
    let rd = std::fs::read_dir(dir)?;
    let mut paths = Vec::new();
    for entry in rd {
        paths.push(entry?.path());
    }
    Ok(paths)
}

/// Parse the `created_ms` prefix from a segment filename like `1234567890-ULID.seg`.
fn parse_created_ms(path: &Path) -> Option<i64> {
    let stem = path.file_stem()?.to_str()?;
    let ms_str = stem.split('-').next()?;
    ms_str.parse::<i64>().ok()
}

/// Build a `Segment` from a path, if the filename matches the expected pattern.
fn segment_from_path(path: &Path, media: Media) -> Option<Segment> {
    let stem = path.file_stem()?.to_str()?;
    let mut parts = stem.splitn(2, '-');
    let created_ms: i64 = parts.next()?.parse().ok()?;
    let id = parts.next()?.to_string();
    // Only accept `.seg` files.
    if path.extension()?.to_str()? != "seg" {
        return None;
    }
    Some(Segment {
        id,
        path: path.to_path_buf(),
        created_ms,
        media,
    })
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rat_core::clock::FakeClock;
    use tempfile::TempDir;

    fn make_writer(dir: &TempDir, ttl_secs: u64, clock: Arc<dyn Clock>) -> RingWriter {
        RingWriter {
            dir: dir.path().to_path_buf(),
            segment_secs: 10,
            ttl_secs,
            clock,
        }
    }

    #[test]
    fn write_then_read_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let clock = FakeClock::at(1_000_000);
        let writer = make_writer(&tmp, 120, clock);
        let key = RingKey::ephemeral();

        let data = b"captured frame bytes";
        let seg = writer.write_segment(Media::Screen, data, &key).unwrap();
        let recovered = writer.read_segment(&seg, &key).unwrap();
        assert_eq!(recovered, data);
    }

    #[test]
    fn segment_file_exists_on_disk() {
        let tmp = TempDir::new().unwrap();
        let clock = FakeClock::at(1_000_000);
        let writer = make_writer(&tmp, 120, clock);
        let key = RingKey::ephemeral();
        let seg = writer.write_segment(Media::Audio, b"audio", &key).unwrap();
        assert!(seg.path.exists());
    }

    #[test]
    fn list_segments_returns_sorted() {
        let tmp = TempDir::new().unwrap();
        let clock = FakeClock::at(0);
        let writer = make_writer(&tmp, 9999, clock.clone());
        let key = RingKey::ephemeral();

        for i in 0..5 {
            clock.advance(10_000);
            writer
                .write_segment(Media::Screen, &[i as u8; 4], &key)
                .unwrap();
        }

        let segs = writer.list_segments(Media::Screen).unwrap();
        assert_eq!(segs.len(), 5);
        // sorted ascending
        for w in segs.windows(2) {
            assert!(w[0].created_ms <= w[1].created_ms);
        }
    }

    #[test]
    fn prune_removes_old_segments_only() {
        // ttl = 120 s; write 30 segments every 10 s (t=10..300 s);
        // prune at t=300 000 ms → cutoff = 300 000 - 120 000 = 180 000 ms;
        // segments at t <= 170 000 ms are deleted, t >= 180 000 ms survive.
        let tmp = TempDir::new().unwrap();
        let clock = FakeClock::at(0);
        let writer = make_writer(&tmp, 120, clock.clone());
        let key = RingKey::ephemeral();

        for _ in 0..30 {
            clock.advance(10_000); // 10 s steps
            writer.write_segment(Media::Screen, b"data", &key).unwrap();
        }
        // clock is now at 300 000 ms; cutoff = 180 000 ms
        // segments created at t=10_000..170_000 are pruned (17 segments)
        // segments at t=180_000..300_000 survive (13 segments)
        let deleted = writer.prune().unwrap();
        assert_eq!(deleted, 17, "expected 17 old segments deleted");

        let surviving = writer.list_segments(Media::Screen).unwrap();
        assert_eq!(surviving.len(), 13, "expected 13 segments to survive");
        // All survivors must have created_ms >= 180_000
        for s in &surviving {
            assert!(
                s.created_ms >= 180_000,
                "stale segment survived: {:?}",
                s.created_ms
            );
        }
    }

    #[test]
    fn list_segments_empty_when_no_media_dir() {
        let tmp = TempDir::new().unwrap();
        let clock = FakeClock::at(0);
        let writer = make_writer(&tmp, 60, clock);
        let segs = writer.list_segments(Media::Clipboard).unwrap();
        assert!(segs.is_empty());
    }
}
