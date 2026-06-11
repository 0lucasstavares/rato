/// New ULID as a 26-char Crockford base32 string. Sortable by creation time.
pub fn new_id() -> String {
    ulid::Ulid::new().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_26_chars_and_unique() {
        let a = new_id();
        let b = new_id();
        assert_eq!(a.len(), 26);
        assert_ne!(a, b);
    }
}
