use sha2::{Digest as _, Sha256};

// sha256(job_id NUL identity) so every replica with the same identity agrees without coordinating.
#[must_use]
pub fn offset_seconds(job_id: &str, identity: &str, window_seconds: u32) -> u32 {
    if window_seconds == 0 {
        return 0;
    }
    let mut hasher = Sha256::new();
    hasher.update(job_id.as_bytes());
    hasher.update([0_u8]);
    hasher.update(identity.as_bytes());
    let digest = hasher.finalize();
    let value = digest
        .iter()
        .take(8)
        .fold(0_u64, |acc, byte| (acc << 8_u32) | u64::from(*byte));
    let remainder = value.checked_rem(u64::from(window_seconds)).unwrap_or(0);
    u32::try_from(remainder).unwrap_or(u32::MAX)
}

mod tests;
