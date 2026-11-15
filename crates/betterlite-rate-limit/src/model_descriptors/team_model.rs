use betterlite_auth::auth_limit_inputs::account::account_limits;
use betterlite_auth::auth_limit_inputs::key_limits::{
    ModelRateLimitKey, get_key_own_model_rate_limit,
};
use betterlite_auth::auth_limit_inputs::metadata_limits::{
    MetadataAccessor, model_limit_from_metadata,
};
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;

use crate::limiter_records::{RateLimitDescriptor, RateLimitedModel};

use super::{ModelDescriptorBuilder, NONE_TEXT, descriptor, window_rate_limit};

impl ModelDescriptorBuilder {
    pub fn team_model_descriptor(
        &self,
        auth: &UserAPIKeyAuth,
        requested_model: Option<&str>,
        out: &mut Vec<RateLimitDescriptor>,
    ) {
        let Some(model) = self.rated_model(requested_model) else {
            return;
        };
        let requests = inherited_team_limit(auth, &model, ModelRateLimitKey::Rpm);
        let tokens = inherited_team_limit(auth, &model, ModelRateLimitKey::Tpm);
        if requests.is_none() && tokens.is_none() {
            return;
        }
        let value = format!(
            "{}:{}",
            account_limits(auth).team_id.unwrap_or(NONE_TEXT),
            model.group
        );
        out.push(descriptor(
            "model_per_team",
            value,
            window_rate_limit(requests, tokens, self.window_size),
        ));
    }
}

fn inherited_team_limit(
    auth: &UserAPIKeyAuth,
    model: &RateLimitedModel,
    rate_limit_key: ModelRateLimitKey,
) -> Option<i64> {
    let team_limit = model
        .limit_in(model_limit_from_metadata(auth, MetadataAccessor::Team, rate_limit_key).as_ref());
    let key_owns_limit = model
        .limit_in(get_key_own_model_rate_limit(auth, rate_limit_key).as_ref())
        .is_some();
    if team_limit.is_none() || key_owns_limit {
        return None;
    }
    team_limit
}
