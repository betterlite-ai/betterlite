use blake2::Blake2b;
use blake2::digest::Digest as _;
use blake2::digest::consts::U8;

// LiteLLM hashes the bare "{user_id}:{server_id}" text, with no domain prefix, as a signed big-endian i64
#[must_use]
pub fn user_env_vars_lock_key(user_id: &str, server_id: &str) -> i64 {
    let mut hasher = Blake2b::<U8>::new();
    hasher.update(user_id.as_bytes());
    hasher.update(b":");
    hasher.update(server_id.as_bytes());
    let mut key = [0_u8; 8];
    key.copy_from_slice(&hasher.finalize());
    i64::from_be_bytes(key)
}
