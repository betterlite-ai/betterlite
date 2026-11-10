use crate::types::control::DynamicCacheControl;

// get_cache reads s-maxage before s-max-age; async_get_cache reads s-max-age before s-maxage.
#[must_use]
pub const fn sync_max_age(control: &DynamicCacheControl) -> Option<i64> {
    match (control.s_maxage, control.s_max_age) {
        (Some(joined), _) => Some(joined),
        (None, hyphenated) => hyphenated,
    }
}

#[must_use]
pub const fn async_max_age(control: &DynamicCacheControl) -> Option<i64> {
    match (control.s_max_age, control.s_maxage) {
        (Some(hyphenated), _) => Some(hyphenated),
        (None, joined) => joined,
    }
}

// The cache control ttl wins over Cache.ttl, and an absent ttl leaves the backend's own default in place.
#[must_use]
pub const fn write_ttl(settings_ttl: Option<i64>, control: &DynamicCacheControl) -> Option<i64> {
    match (control.ttl, settings_ttl) {
        (Some(control_ttl), _) => Some(control_ttl),
        (None, fallback) => fallback,
    }
}
