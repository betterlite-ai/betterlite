use super::payload::{Metadata, Payload};
use super::timestamp::Timestamp;
use crate::otel::model::utils::coercion::OtelModelValue;
use serde_with::{As, DefaultOnNull};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct Callback {
    pub litellm_call_id: Option<String>,
    pub model: Option<String>,
    pub call_type: Option<String>,
    pub standard_logging_object: Option<Payload>,
    #[serde(rename = "standard_callback_dynamic_params")]
    pub dynamic_params: Option<OtelModelValue>,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub litellm_no_upstream_llm_call: bool,
    pub api_call_start_time: Option<Timestamp>,
    pub completion_start_time: Option<Timestamp>,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub optional_params: OptionalParams,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub litellm_params: Params,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct Params {
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub metadata: Metadata,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub litellm_metadata: Metadata,
    pub litellm_session_id: Option<String>,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub proxy_server_request: ProxyRequest,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct ProxyRequest {
    pub headers: Option<BTreeMap<String, Option<String>>>,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct OptionalParams {
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub stream: bool,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct RequestData {
    pub model: Option<String>,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub metadata: Metadata,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub litellm_metadata: Metadata,
}
