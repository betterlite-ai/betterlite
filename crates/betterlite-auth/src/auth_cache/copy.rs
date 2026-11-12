use crate::resolved_key::auth::UserAPIKeyAuth;

// model_copy is shallow, so a clone keeps the trusted markers and shares every immutable nested record
#[must_use]
pub fn copy_for_cache(auth: &UserAPIKeyAuth) -> UserAPIKeyAuth {
    let mut copied = auth.clone();
    copied.extra.budget_reservation = None;
    copied.extra.budget_throttle_pct = None;
    copied.extra.parent_otel_span = None;
    copied.extra.request_route = None;
    copied
}
