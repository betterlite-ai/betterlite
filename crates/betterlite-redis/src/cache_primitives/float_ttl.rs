mod tests;

/// The second pipelined command `_incrbyfloat_with_ttl` issues alongside `INCRBYFLOAT`.
///
/// Chosen once per call so the whole round trip stays a single pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TtlFollowup {
    Ttl,
    Expire,
}

/// `None` means the call has no TTL at all and only `INCRBYFLOAT` is issued.
#[must_use]
pub const fn plan_ttl_followup(ttl: Option<i64>, refresh_ttl: bool) -> Option<TtlFollowup> {
    match ttl {
        None => None,
        Some(_) if refresh_ttl => Some(TtlFollowup::Expire),
        Some(_) => Some(TtlFollowup::Ttl),
    }
}

/// True when the pipelined `TTL` reply shows the key has no expiration.
///
/// A third, standalone `EXPIRE` call is still needed to give it one.
#[must_use]
pub fn needs_standalone_expire(followup: Option<TtlFollowup>, ttl_reply: i64) -> bool {
    followup == Some(TtlFollowup::Ttl) && ttl_reply == -1
}
