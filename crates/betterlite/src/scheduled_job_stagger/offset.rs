use core::num::{NonZeroU32, NonZeroU64};

use sha2::{Digest as _, Sha256};

// sha256(job_id NUL identity) so every replica with the same identity agrees without coordinating.
#[must_use]
pub fn offset_seconds(job_id: &str, identity: &str, window_seconds: u32) -> u32 {
    let Some(window) = NonZeroU32::new(window_seconds) else {
        return 0;
    };
    let mut hasher = Sha256::new();
    hasher.update(job_id.as_bytes());
    hasher.update([0_u8]);
    hasher.update(identity.as_bytes());
    let bytes = <[u8; 32]>::from(hasher.finalize());
    let [
        first,
        second,
        third,
        fourth,
        fifth,
        sixth,
        seventh,
        eighth,
        ..,
    ] = bytes;
    let digest_prefix =
        u64::from_be_bytes([first, second, third, fourth, fifth, sixth, seventh, eighth]);
    let remainder = digest_prefix % NonZeroU64::from(window);
    // remainder < u64::from(window.get()), itself a u32, so the fallback branch is unreachable.
    u32::try_from(remainder).unwrap_or_else(|_err| window.get().wrapping_sub(1))
}

mod tests;
