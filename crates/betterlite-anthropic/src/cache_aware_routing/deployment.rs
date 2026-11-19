use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

// The deployment fields native prediction reads; LiteLLM's Deployment carries the same id, blocked flag and params.
#[derive(Clone, Copy, Debug)]
pub struct PredictionDeployment<'deployment> {
    pub deployment_id: &'deployment str,
    pub model: Option<&'deployment str>,
    pub blocked: bool,
    pub litellm_params: &'deployment JsonObject,
    pub model_info: &'deployment JsonObject,
}

impl<'deployment> PredictionDeployment<'deployment> {
    #[must_use]
    pub fn read(
        litellm_params: &'deployment JsonObject,
        model_info: &'deployment JsonObject,
    ) -> Self {
        Self {
            deployment_id: text_field(model_info, "id").unwrap_or_default(),
            model: text_field(litellm_params, "model"),
            blocked: matches!(model_info.get("blocked"), Some(Json::Bool(true))),
            litellm_params,
            model_info,
        }
    }
}

// One arm's request: the deployment, the caller's key hash and the instant its observation lookup reads.
#[derive(Clone, Copy, Debug)]
pub struct ArmRequest<'request> {
    pub deployment: PredictionDeployment<'request>,
    pub caller_key_hash: &'request str,
    pub checked_at: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct WarmRequest<'request> {
    pub deployment: PredictionDeployment<'request>,
    pub caller_key_hash: &'request str,
    pub now: f64,
}

fn text_field<'maps>(map: &'maps JsonObject, field: &str) -> Option<&'maps str> {
    match map.get(field) {
        Some(Json::Text(text)) => Some(text),
        _ => None,
    }
}
