use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

use super::exception::ExceptionInput;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StatusCodeInput {
    pub(crate) enum_status_code: Option<String>,
    pub(crate) exception: Option<ExceptionInput>,
    pub(crate) kwargs_exception: Option<ExceptionInput>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InvalidKeyInput {
    pub(crate) status_code: Option<i64>,
    pub(crate) exception: Option<ExceptionInput>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExceptionCaseInput {
    pub(crate) exception: ExceptionInput,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeploymentLabelsInput {
    pub(crate) payload_metadata: Option<IndexMap<String, Json>>,
    pub(crate) litellm_params_metadata: Option<IndexMap<String, Json>>,
    pub(crate) user_api_key_auth: Option<AuthInput>,
}

// the UserAPIKeyAuth attributes _extract_deployment_failure_label_values reaches through getattr
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthInput {
    pub(crate) api_key: Option<String>,
    pub(crate) user_email: Option<String>,
    pub(crate) key_alias: Option<String>,
    pub(crate) team_id: Option<String>,
    pub(crate) team_alias: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BoundedModelInput {
    pub(crate) requested_model: Option<Json>,
    #[serde(default)]
    pub(crate) router_originated: bool,
    pub(crate) catalog: Option<CatalogInput>,
}

// the proxy router _bounded_requested_model_label asks; its presence means llm_router exists
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CatalogInput {
    #[serde(default)]
    pub(crate) recognized: Vec<String>,
    #[serde(default)]
    pub(crate) team_public: Vec<String>,
    #[serde(default)]
    pub(crate) patterns: Vec<String>,
    #[serde(default)]
    pub(crate) team_patterns: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RateLimitInput {
    pub(crate) exception: Option<ExceptionInput>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderRequestInput {
    pub(crate) litellm_params: Option<IndexMap<String, Json>>,
    pub(crate) standard_logging: Option<IndexMap<String, Json>>,
    pub(crate) model: Option<String>,
}
