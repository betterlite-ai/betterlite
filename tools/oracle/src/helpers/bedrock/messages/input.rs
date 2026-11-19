use std::collections::BTreeMap;

use betterlite_bedrock::event_frames::beta::BetaHeader;
use betterlite_bedrock::invoke_primitives::config::Params;
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as MessageJson;
use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    InvokeUrl(Box<InvokeUrl>),
    Transform(Transform),
    EventStream(EventStream),
    ErrorClass(ErrorClass),
    Constructor(Constructor),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InvokeUrl {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) stream: Option<bool>,
    #[serde(default)]
    pub(crate) optional_params: Object,
    #[serde(default)]
    pub(crate) headers: IndexMap<String, String>,
    #[serde(default)]
    pub(crate) metadata: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(crate) spend_logs_metadata: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(crate) policy_fields: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) api_key: Option<SecretString>,
    #[serde(default)]
    pub(crate) configured_bearer_token: Option<SecretString>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) credentials: Keys,
    #[serde(default)]
    pub(crate) request_data: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) fake_stream: Option<bool>,
    pub(crate) now: i64,
}

impl InvokeUrl {
    // The invoke configs take Box<RawValue>, which serde's Content buffering cannot rebuild, so the case sends plain JSON.
    pub(crate) fn invoke_params(&self) -> Result<Params, serde_json::Error> {
        let plain = serde_json::to_value(&self.optional_params)?;
        let params: Params = serde_json::from_value(plain)?;
        Ok(params)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Keys {
    pub(crate) access_key_id: SecretString,
    pub(crate) secret_access_key: SecretString,
    #[serde(default)]
    pub(crate) session_token: Option<SecretString>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Transform {
    pub(crate) model_cost: ParamMap,
    #[serde(default)]
    pub(crate) model_info: BTreeMap<String, IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) drop_params: bool,
    pub(crate) calls: Vec<TransformCall>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransformCall {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) messages: Vec<MessageJson>,
    #[serde(default)]
    pub(crate) optional_params: Object,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, BetaHeader>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EventStream {
    pub(crate) chunks: Vec<String>,
    #[serde(default)]
    pub(crate) response_headers: Option<BTreeMap<String, String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorClass {
    pub(crate) message: String,
    pub(crate) status_code: u16,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Constructor {
    #[serde(default)]
    pub(crate) kwargs: Object,
}
