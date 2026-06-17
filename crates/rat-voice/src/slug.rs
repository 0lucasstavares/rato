use sha2::{Digest, Sha256};

const ADJECTIVES: &[&str] = &[
    "amber", "brisk", "calm", "daring", "ember", "fuzzy", "glossy", "honest", "iron", "jolly",
    "kind", "lunar", "mellow", "nimble", "opal", "plain",
];

const NOUNS: &[&str] = &[
    "anchor", "bolt", "comet", "drift", "echo", "flare", "glade", "harbor", "island", "jacket",
    "kite", "lantern", "magnet", "notch", "orbit", "pixel",
];

pub fn spoken_slug(id: &str) -> String {
    let digest = Sha256::digest(id.as_bytes());
    let a = usize::from(digest[0]) % ADJECTIVES.len();
    let n = usize::from(digest[1]) % NOUNS.len();
    format!("{}-{}", ADJECTIVES[a], NOUNS[n])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_is_deterministic_and_two_words() {
        let a = spoken_slug("01KTYVOICEAPPROVAL");
        let b = spoken_slug("01KTYVOICEAPPROVAL");
        assert_eq!(a, b);
        assert_eq!(a.split('-').count(), 2);
    }

    #[test]
    fn slug_changes_for_different_ids() {
        assert_ne!(spoken_slug("approval-a"), spoken_slug("approval-b"));
    }
}
