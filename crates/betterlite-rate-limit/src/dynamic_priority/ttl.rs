use super::settings::PriorityPolicy;

mod tests;

// _get_saturation_check_cache_ttl: how long the usage cache keeps a saturation counter before it reads Redis again
#[must_use]
// ast-grep-ignore: no-one-line-functions
pub const fn saturation_check_cache_ttl(policy: &PriorityPolicy) -> i64 {
    policy.settings.saturation_check_cache_ttl
}
