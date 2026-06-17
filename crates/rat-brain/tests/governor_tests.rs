use rat_brain::governor::Governor;

#[test]
fn burst_then_deny() {
    let mut gov = Governor::new();
    // mentor starts with 2 tokens
    assert!(gov.admit("mentor", 1_000), "first admit should succeed");
    assert!(gov.admit("mentor", 2_000), "second admit should succeed");
    // bucket now empty
    assert!(
        !gov.admit("mentor", 3_000),
        "third admit should be denied (bucket empty)"
    );
}

#[test]
fn refill_after_30_min() {
    let mut gov = Governor::new();
    // drain mentor bucket (2 tokens)
    assert!(gov.admit("mentor", 1_000));
    assert!(gov.admit("mentor", 2_000));
    // advance 30 min (1_800_000 ms) → should refill 1 token
    assert!(
        gov.admit("mentor", 1_000 + 1_800_000),
        "should admit after 30 min refill"
    );
}

#[test]
fn global_cap_8_per_hour() {
    // Strategy: admit 7 chaos tokens (spaced 600_001 ms apart for clean refills),
    // then 1 mentor token — all within a 1-hour window.
    // The 9th admit should be denied by the global cap.
    //
    // chaos admits: t=0 (tok=1), t=1 (tok=0),
    //   t=600_001 (refill→1→0), t=1_200_001, t=1_800_001, t=2_400_001, t=3_000_001
    // mentor admit: t=3_100_000 (fresh bucket, tok=2→1, global=8)
    // 9th at t=3_100_001: one_hour_ago = -499_999, all 8 retained → deny
    let mut gov = Governor::new();
    assert!(gov.admit("chaos", 0)); // global=1
    assert!(gov.admit("chaos", 1)); // global=2, chaos tok=0
    assert!(gov.admit("chaos", 600_001)); // refill, global=3
    assert!(gov.admit("chaos", 1_200_001)); // refill, global=4
    assert!(gov.admit("chaos", 1_800_001)); // refill, global=5
    assert!(gov.admit("chaos", 2_400_001)); // refill, global=6
    assert!(gov.admit("chaos", 3_000_001)); // refill, global=7
    assert!(gov.admit("mentor", 3_100_000)); // mentor fresh, tok=2→1, global=8
                                             // 9th: all 8 are within 1 hour of t=3_100_001 → global cap hit
    assert!(
        !gov.admit("mentor", 3_100_001),
        "9th admit should be denied by global cap"
    );
}

#[test]
fn quiet_mode_single_token() {
    let mut gov = Governor::new();
    // quiet mode has capacity=1
    assert!(
        gov.admit("quiet", 1_000),
        "first quiet admit should succeed"
    );
    assert!(
        !gov.admit("quiet", 2_000),
        "second quiet admit should fail (capacity=1)"
    );
}

#[test]
fn chaos_mode_refills_faster() {
    let mut gov = Governor::new();
    // chaos refills 1 token per 600_000 ms (10 min)
    // First two admits drain the 2-token capacity
    assert!(gov.admit("chaos", 0));
    assert!(gov.admit("chaos", 1));
    // After 499_999 ms from last admit: accrued < 1 → deny
    // (from t=1 to t=500_000 = 499_999 ms, accrued = 499_999/600_000 < 1)
    assert!(!gov.admit("chaos", 500_000));
    // After 600_000 ms from last admit (t=1 + 600_000 = 600_001):
    // accrued = 600_000/600_000 = 1.0 → admit ✓
    // BUT: the failed call at t=500_000 updated last_refill_ms to 500_000
    // so from t=500_000 we need 600_000 more ms: t=500_000 + 600_000 = 1_100_000
    assert!(
        gov.admit("chaos", 1_100_000),
        "should admit after 600_000 ms from last refill update"
    );
}

#[test]
fn dedupe_key_sorted_deterministic() {
    let ids = vec!["c".to_string(), "a".to_string(), "b".to_string()];
    let key = Governor::dedupe_key(&ids);
    assert_eq!(key, "a|b|c");

    // Same IDs in different order → same key
    let ids2 = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    assert_eq!(Governor::dedupe_key(&ids2), key);
}

#[test]
fn unknown_mode_falls_back_to_mentor() {
    let mut gov = Governor::new();
    // "unknown" mode falls back to mentor bucket (capacity=2)
    assert!(gov.admit("unknown_mode", 1_000));
    assert!(gov.admit("unknown_mode", 2_000));
    assert!(!gov.admit("unknown_mode", 3_000));
}
