use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IoLimits {
    pub itpm: Option<i64>,
    pub otpm: Option<i64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IoDeployment<'deployment> {
    pub model_id: Option<&'deployment str>,
    pub model: Option<&'deployment str>,
    pub limits: IoLimits,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MinuteKeys {
    pub itpm: String,
    pub otpm: String,
}

// LiteLLM's _get_cache_keys refuses to build a shared "None:None" bucket for a half-configured deployment
#[must_use]
pub fn minute_keys(deployment: &IoDeployment<'_>, minute: &str) -> Option<MinuteKeys> {
    let model_id = deployment.model_id?;
    let model = deployment.model?;
    Some(MinuteKeys {
        itpm: format!("global_router:{model_id}:{model}:itpm:{minute}"),
        otpm: format!("global_router:{model_id}:{model}:otpm:{minute}"),
    })
}
