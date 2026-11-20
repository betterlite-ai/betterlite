use betterlite_auth::auth_limit_inputs::account::account_limits;
use betterlite_auth::auth_limit_inputs::metadata_limits::get_key_tag_rpm_limit;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::api::request_destination::RequestData;

use super::descriptor::rate_limit_descriptor;
use super::tag_fromkeys::deduplicated_tags;
use crate::limiter_records::RateLimitDescriptor;

mod tests;

// LiteLLM's _add_tag_per_key_rate_limit_descriptor: one {api_key}:{tag} RPM bucket per configured request tag
pub fn add_tag_per_key_descriptor(
    auth: &UserAPIKeyAuth,
    data: &RequestData,
    window_size: i64,
    out: &mut Vec<RateLimitDescriptor>,
) {
    let Some(api_key) = account_limits(auth).hashed_api_key else {
        return;
    };
    let Some(tag_rpm_limits) = get_key_tag_rpm_limit(auth) else {
        return;
    };
    for tag in deduplicated_tags(data) {
        let Some(rpm_limit) = tag_rpm_limits.get(&tag) else {
            continue;
        };
        out.push(rate_limit_descriptor(
            "tag_per_key",
            format!("{api_key}:{tag}"),
            Some(*rpm_limit),
            None,
            window_size,
        ));
    }
}
