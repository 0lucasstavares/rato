use std::collections::VecDeque;

#[derive(Debug, Clone, PartialEq)]
pub struct PcmFrame {
    pub captured_ms: i64,
    pub samples: Vec<f32>,
}

/// RAM-only pre-wake audio ring. It intentionally exposes no serialization or
/// file-writing API; callers can only push samples, inspect current RAM length,
/// and drain/clear RAM after wake handling.
#[derive(Debug)]
pub struct PreWakeRing {
    max_samples: usize,
    samples: VecDeque<f32>,
}

impl PreWakeRing {
    pub fn new(sample_rate_hz: usize, seconds: usize) -> Self {
        Self {
            max_samples: sample_rate_hz.saturating_mul(seconds).max(1),
            samples: VecDeque::new(),
        }
    }

    pub fn push_frame(&mut self, frame: &[f32]) {
        for sample in frame {
            self.samples.push_back(*sample);
            while self.samples.len() > self.max_samples {
                self.samples.pop_front();
            }
        }
    }

    pub fn len_samples(&self) -> usize {
        self.samples.len()
    }

    pub fn snapshot(&self) -> Vec<f32> {
        self.samples.iter().copied().collect()
    }

    pub fn clear(&mut self) {
        for sample in &mut self.samples {
            *sample = 0.0;
        }
        self.samples.clear();
    }
}

impl Drop for PreWakeRing {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::{AudioSource, FakeAudioSource, FakeWakeDetector, WakeDetector};

    #[test]
    fn ring_is_bounded_to_configured_window() {
        let mut ring = PreWakeRing::new(4, 2);
        ring.push_frame(&[1.0, 2.0, 3.0, 4.0]);
        ring.push_frame(&[5.0, 6.0, 7.0, 8.0]);
        ring.push_frame(&[9.0, 10.0]);
        assert_eq!(ring.len_samples(), 8);
        assert_eq!(
            ring.snapshot(),
            vec![3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]
        );
    }

    #[test]
    fn clear_removes_samples() {
        let mut ring = PreWakeRing::new(16_000, 8);
        ring.push_frame(&[1.0, 2.0]);
        ring.clear();
        assert_eq!(ring.len_samples(), 0);
    }

    #[test]
    fn fake_prewake_path_does_not_write_to_watched_dirs() {
        let state = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let mut source = FakeAudioSource::new(vec![vec![0.1; 160], vec![0.2; 160], vec![0.3; 160]]);
        let mut wake = FakeWakeDetector::fire_on(2, "hey rat");
        let mut ring = PreWakeRing::new(16_000, 8);

        while let Some(frame) = source.next_frame().unwrap() {
            ring.push_frame(&frame);
            if wake.detect(&frame).unwrap().is_some() {
                break;
            }
        }

        assert!(std::fs::read_dir(state.path()).unwrap().next().is_none());
        assert!(std::fs::read_dir(data.path()).unwrap().next().is_none());
        assert!(ring.len_samples() > 0);
    }
}
