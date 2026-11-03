use super::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use rust_decimal::Decimal;
use serde_with::{As, DefaultOnNull};

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct Payload {
    pub litellm_call_id: Option<String>,
    pub id: Option<String>,
    pub model: Option<String>,
    pub end_user: Option<String>,
    pub model_group: Option<String>,
    pub model_id: Option<String>,
    pub api_base: Option<String>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub response_cost: Option<Decimal>,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub metadata: Metadata,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub hidden_params: Hidden,
    #[serde(default)]
    pub response: Response,
    // Span builders consume payload fields outside this identity projection.
    #[serde(flatten)]
    pub extra: MetadataMap,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct Metadata {
    pub user_api_key_team_id: Option<String>,
    pub team_id: Option<String>,
    pub user_api_key_team_alias: Option<String>,
    pub team_alias: Option<String>,
    pub user_api_key_hash: Option<String>,
    pub user_api_key_user_id: Option<String>,
    pub user_api_key_org_id: Option<String>,
    pub user_api_key_alias: Option<String>,
    pub requester_ip_address: Option<String>,
    pub user_api_key_end_user_id: Option<String>,
    pub user_api_key_request_route: Option<String>,
    pub deployment: Option<String>,
    pub model_group: Option<String>,
    pub session_id: Option<String>,
    pub trace_id: Option<String>,
    pub trace_name: Option<String>,
    pub trace_user_id: Option<String>,
    pub user_api_key_team_metadata: Option<MetadataMap>,
    pub user_api_key_auth_metadata: Option<MetadataMap>,
    pub requester_metadata: Option<MetadataMap>,
    pub model_info: Option<ModelInfo>,
    pub litellm_session_id_generated: Option<bool>,
    pub tags: Option<Vec<OtelModelValue>>,
    #[serde(flatten)]
    pub extra: MetadataMap,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct ModelInfo {
    pub id: Option<String>,
    #[serde(flatten)]
    pub extra: MetadataMap,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct Hidden {
    pub litellm_model_name: Option<String>,
    pub api_base: Option<String>,
    #[serde(flatten)]
    pub extra: MetadataMap,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(untagged)]
pub enum Response {
    Object {
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        // Provider-specific content remains available to downstream span builders.
        #[serde(flatten)]
        extra: MetadataMap,
    },
    Text(String),
    Items(Vec<OtelModelValue>),
    #[default]
    Null,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct Auth {
    pub team_id: Option<String>,
    pub team_alias: Option<String>,
    pub team_metadata: Option<MetadataMap>,
    // UserAPIKeyAuth hashes api_key before this projection receives it.
    #[serde(rename = "api_key")]
    pub key_hash: Option<String>,
    pub end_user_id: Option<String>,
    pub user_id: Option<String>,
    pub org_id: Option<String>,
    pub key_alias: Option<String>,
}
