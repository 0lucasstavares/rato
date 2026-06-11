/// Token bucket governor for LLM pushback rate limiting.
/// Deterministic — caller passes now_ms, no SystemTime.
pub struct Governor {
    mentor: BucketState,
    chaos: BucketState,
    quiet: BucketState,
    /// Global sliding window: timestamps of admitted calls in last hour.
    global_window: Vec<i64>,
}

struct BucketState {
    tokens: f64,
    last_refill_ms: Option<i64>,
    capacity: f64,
    /// ms per token
    refill_ms: f64,
}

impl BucketState {
    fn new(capacity: f64, refill_per_ms: f64) -> Self {
        Self {
            tokens: capacity,
            last_refill_ms: None,
            capacity,
            refill_ms: refill_per_ms,
        }
    }

    fn refill(&mut self, now_ms: i64) {
        match self.last_refill_ms {
            None => {
                // First call: record time, bucket starts at full capacity.
                self.last_refill_ms = Some(now_ms);
            }
            Some(last) => {
                let elapsed = (now_ms - last).max(0) as f64;
                let accrued = elapsed / self.refill_ms;
                self.tokens = (self.tokens + accrued).min(self.capacity);
                self.last_refill_ms = Some(now_ms);
            }
        }
    }

    fn try_consume(&mut self) -> bool {
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

impl Governor {
    pub fn new() -> Self {
        // mentor: capacity 2, refill 1 per 30 min = 1/1_800_000 ms
        // chaos:  capacity 2, refill 1 per 10 min = 1/600_000 ms
        // quiet:  capacity 1, refill 1 per 120 min = 1/7_200_000 ms
        Self {
            mentor: BucketState::new(2.0, 1_800_000.0),
            chaos: BucketState::new(2.0, 600_000.0),
            quiet: BucketState::new(1.0, 7_200_000.0),
            global_window: Vec::new(),
        }
    }

    pub fn admit(&mut self, mode: &str, now_ms: i64) -> bool {
        // Prune global window to last hour
        let one_hour_ago = now_ms - 3_600_000;
        self.global_window.retain(|&ts| ts > one_hour_ago);

        // Check global cap: 8/h
        if self.global_window.len() >= 8 {
            return false;
        }

        let bucket = match mode {
            "mentor" => &mut self.mentor,
            "chaos" => &mut self.chaos,
            "quiet" => &mut self.quiet,
            _ => &mut self.mentor,
        };

        bucket.refill(now_ms);
        if bucket.try_consume() {
            self.global_window.push(now_ms);
            true
        } else {
            false
        }
    }

    /// Canonical dedupe key for a set of evidence IDs.
    /// Returns sorted IDs joined with "|".
    pub fn dedupe_key(evidence_ids: &[String]) -> String {
        let mut sorted = evidence_ids.to_vec();
        sorted.sort();
        sorted.join("|")
    }
}

impl Default for Governor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_then_deny() {
        let mut gov = Governor::new();
        // mentor starts with 2 tokens
        assert!(gov.admit("mentor", 1_000));
        assert!(gov.admit("mentor", 2_000));
        // bucket now empty
        assert!(!gov.admit("mentor", 3_000));
    }

    #[test]
    fn refill_after_30_min() {
        let mut gov = Governor::new();
        // drain mentor bucket
        assert!(gov.admit("mentor", 1_000));
        assert!(gov.admit("mentor", 2_000));
        // advance 30 min → 1 token refilled
        assert!(gov.admit("mentor", 1_000 + 1_800_000));
    }

    #[test]
    fn global_cap_8_per_hour() {
        // Strategy: use chaos (capacity=2, 600_000 ms/token) for 7 admits packed within
        // a tight range, then use mentor for the 8th admit — all within a 1-hour window.
        // Then the 9th admit should be denied by the global cap.
        //
        // chaos admits at: t=0 (tok=1), t=1 (tok=0),
        //   t=600_001 (refill→1, tok→0), t=1_200_001, t=1_800_001, t=2_400_001, t=3_000_001
        // mentor admit at: t=3_100_000 (mentor starts fresh, capacity=2, tok→1)
        // All 8 within [0, 3_100_000] which is < 3_600_000. Window for 9th covers all 8.
        // 9th at t=3_100_001: one_hour_ago = 3_100_001 - 3_600_000 = -499_999
        //   retain ts > -499_999: all 8 entries kept → global=8 → deny ✓
        let mut gov = Governor::new();
        assert!(gov.admit("chaos", 0));             // global=1
        assert!(gov.admit("chaos", 1));             // global=2, chaos tok=0
        assert!(gov.admit("chaos", 600_001));       // refill 1, global=3
        assert!(gov.admit("chaos", 1_200_001));     // refill 1, global=4
        assert!(gov.admit("chaos", 1_800_001));     // refill 1, global=5
        assert!(gov.admit("chaos", 2_400_001));     // refill 1, global=6
        assert!(gov.admit("chaos", 3_000_001));     // refill 1, global=7
        assert!(gov.admit("mentor", 3_100_000));    // mentor fresh, tok=1, global=8
        // 9th: all 8 entries are within 1 hour of t=3_100_001 → deny
        assert!(!gov.admit("mentor", 3_100_001), "9th admit should be denied by global cap");
    }

    #[test]
    fn dedupe_key_is_sorted() {
        let ids = vec!["c".to_string(), "a".to_string(), "b".to_string()];
        assert_eq!(Governor::dedupe_key(&ids), "a|b|c");
    }

    #[test]
    fn dedupe_key_empty() {
        let ids: Vec<String> = vec![];
        assert_eq!(Governor::dedupe_key(&ids), "");
    }
}
