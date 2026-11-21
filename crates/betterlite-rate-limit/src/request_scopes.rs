use betterlite_core::api::request_destination::RequestData;
use betterlite_models::decisions::Json;

use crate::identity_descriptors::agent_descriptor::AgentDescriptorBuilder;
use crate::limiter_records::TagRateLimitResolver;
use crate::model_descriptors::ModelDescriptorBuilder;

use self::deployment_health::DeploymentHealthPolicy;

pub mod deployment_health;
pub mod key_scopes;
pub mod limit_policy;
pub mod request_assembly;

pub(crate) mod tests;

// a text member of the request body: a missing, null or non-text member reads as absent
#[must_use]
pub fn body_text<'body>(data: &'body RequestData, name: &str) -> Option<&'body str> {
    match data.get(name) {
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

// Everything one assembled request needs, so each builder step takes only the request itself
pub struct RequestScopeBuilder<'deps> {
    pub window_size: i64,
    pub model: ModelDescriptorBuilder,
    pub agents: AgentDescriptorBuilder<'deps>,
    pub health: DeploymentHealthPolicy<'deps>,
    pub tag_resolver: &'deps dyn TagRateLimitResolver,
}
