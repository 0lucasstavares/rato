use chacha20poly1305::{
    aead::{Aead, AeadCore, KeyInit, OsRng, Payload},
    XChaCha20Poly1305, XNonce,
};
use thiserror::Error;
use zeroize::Zeroize;

/// Error type for ring-buffer crypto operations.
#[derive(Debug, Error)]
pub enum RingError {
    #[error("AEAD decryption failed (wrong key, tampered ciphertext, or bad AAD)")]
    DecryptFailed,

    #[error("sealed blob is too short to contain a nonce")]
    TooShort,

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

// XChaCha20Poly1305 uses a 24-byte nonce.
const NONCE_LEN: usize = 24;

/// A 32-byte ephemeral symmetric key.
///
/// The key bytes are zeroized on drop.  On supported platforms they are also
/// mlock-ed (best-effort; failures are silently ignored – the zeroize guarantee
/// is the primary security property).
pub struct RingKey {
    mlocked: bool,
    bytes: [u8; 32],
}

impl RingKey {
    /// Generate a fresh ephemeral key using the OS CSPRNG.
    pub fn ephemeral() -> Self {
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        let mlocked = try_mlock(bytes.as_ptr(), bytes.len());
        Self { mlocked, bytes }
    }

    /// Raw key bytes (never log these).
    pub(crate) fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }
}

impl Drop for RingKey {
    fn drop(&mut self) {
        if self.mlocked {
            try_munlock(self.bytes.as_ptr(), self.bytes.len());
        }
        // Explicitly zeroize the key material before the memory is released.
        self.bytes.zeroize();
    }
}

// ── mlock helpers (best-effort; non-fatal) ──────────────────────────────────

fn try_mlock(ptr: *const u8, len: usize) -> bool {
    #[cfg(unix)]
    unsafe {
        libc::mlock(ptr as *const libc::c_void, len) == 0
    }
    #[cfg(not(unix))]
    {
        let _ = (ptr, len);
        false
    }
}

fn try_munlock(ptr: *const u8, len: usize) {
    #[cfg(unix)]
    unsafe {
        libc::munlock(ptr as *const libc::c_void, len);
    }
    #[cfg(not(unix))]
    {
        let _ = (ptr, len);
    }
}

// ── seal / open ─────────────────────────────────────────────────────────────

/// Encrypt `plaintext` with `key`.
///
/// Returns `nonce (24 bytes) || ciphertext+tag`.
/// A fresh random nonce is generated for every call.
pub fn seal(key: &RingKey, plaintext: &[u8], aad: &[u8]) -> Vec<u8> {
    let cipher = XChaCha20Poly1305::new(key.as_bytes().into());
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let payload = Payload { msg: plaintext, aad };
    // encrypt_in_place_detached not needed; just use encrypt which appends the
    // 16-byte Poly1305 tag.
    let ciphertext = cipher
        .encrypt(&nonce, payload)
        .expect("XChaCha20Poly1305 encryption should never fail for valid key/nonce");

    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(nonce.as_slice());
    out.extend_from_slice(&ciphertext);
    out
}

/// Decrypt a blob produced by [`seal`].
pub fn open(key: &RingKey, sealed: &[u8], aad: &[u8]) -> Result<Vec<u8>, RingError> {
    if sealed.len() < NONCE_LEN {
        return Err(RingError::TooShort);
    }
    let (nonce_bytes, ciphertext) = sealed.split_at(NONCE_LEN);
    let nonce = XNonce::from_slice(nonce_bytes);
    let cipher = XChaCha20Poly1305::new(key.as_bytes().into());
    let payload = Payload { msg: ciphertext, aad };
    cipher
        .decrypt(nonce, payload)
        .map_err(|_| RingError::DecryptFailed)
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_roundtrip() {
        let key = RingKey::ephemeral();
        let plain = b"hello ring buffer";
        let aad = b"screen";
        let sealed = seal(&key, plain, aad);
        let recovered = open(&key, &sealed, aad).expect("should decrypt");
        assert_eq!(recovered, plain);
    }

    #[test]
    fn open_wrong_key_fails() {
        let key = RingKey::ephemeral();
        let other = RingKey::ephemeral();
        let sealed = seal(&key, b"secret", b"aad");
        assert!(open(&other, &sealed, b"aad").is_err());
    }

    #[test]
    fn open_wrong_aad_fails() {
        let key = RingKey::ephemeral();
        let sealed = seal(&key, b"secret", b"screen");
        assert!(open(&key, &sealed, b"audio").is_err());
    }

    #[test]
    fn open_tampered_ciphertext_fails() {
        let key = RingKey::ephemeral();
        let mut sealed = seal(&key, b"secret", b"aad");
        // Flip a byte in the ciphertext (past the nonce).
        let last = sealed.len() - 1;
        sealed[last] ^= 0xff;
        assert!(open(&key, &sealed, b"aad").is_err());
    }

    #[test]
    fn two_seals_produce_different_output() {
        let key = RingKey::ephemeral();
        let a = seal(&key, b"same", b"aad");
        let b = seal(&key, b"same", b"aad");
        assert_ne!(a, b, "each seal must use a fresh nonce");
    }

    #[test]
    fn open_too_short_returns_error() {
        let key = RingKey::ephemeral();
        assert!(matches!(open(&key, &[0u8; 5], b""), Err(RingError::TooShort)));
    }
}
