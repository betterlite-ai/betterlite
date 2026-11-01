use super::metadata::Metadata;
use crate::tiered_pricing::Scalar;

const INJECTED_FOR_EVERY_DEPLOYMENT: &str = "";

// An empty marker credits every leg; a nonstring marker never attributes savings.
#[must_use]
pub fn marks_gateway_injection(metadata: Option<&Metadata>, model_id: Option<&str>) -> bool {
    let Some(Scalar::Text(injected_deployment)) =
        metadata.and_then(|value| value.litellm_gateway_injected_cache.as_ref())
    else {
        return false;
    };
    injected_deployment == INJECTED_FOR_EVERY_DEPLOYMENT
        || Some(injected_deployment.as_str()) == model_id
}

mod tests;
