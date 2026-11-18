use betterlite_prompts::arguments::json::Json;

use crate::converse_request::CommonRequest;
use crate::converse_request::convert::request_value;
use crate::request_policy::{BedrockRequestPolicy, Params};

// B596 owns the pair: ownership follows the operator's allow-list, not whether anything resolved.
pub(super) fn resolve(
    policy: Option<&BedrockRequestPolicy>,
    litellm_params: &Params,
    common: &mut CommonRequest,
) {
    let Some(policy) = policy else {
        return;
    };
    let caller = common.request_metadata.as_ref().map(request_value);
    let resolved = policy.resolve(litellm_params, caller.as_ref());
    common.request_metadata = resolved.map(|pairs| {
        Json::Object(
            pairs
                .into_iter()
                .map(|(key, value)| (key, Json::Text(value)))
                .collect(),
        )
    });
}
