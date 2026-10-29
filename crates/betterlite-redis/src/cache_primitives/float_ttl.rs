mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TtlFollowup {
    Ttl,
    Expire,
}

#[must_use]
pub const fn plan_ttl_followup(ttl: Option<i64>, refresh_ttl: bool) -> Option<TtlFollowup> {
    match ttl {
        None => None,
        Some(_) if refresh_ttl => Some(TtlFollowup::Expire),
        Some(_) => Some(TtlFollowup::Ttl),
    }
}

// a standalone EXPIRE is still needed to give the key a TTL it never had
#[must_use]
pub fn needs_standalone_expire(followup: Option<TtlFollowup>, ttl_reply: i64) -> bool {
    if followup == Some(TtlFollowup::Ttl) {
        ttl_reply == -1
    } else {
        false
    }
}
