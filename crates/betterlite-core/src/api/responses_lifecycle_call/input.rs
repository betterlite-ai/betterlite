use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::response_resources::request::InputItemsOrder;
use indexmap::IndexMap;
use secrecy::zeroize::Zeroize;
use secrecy::{CloneableSecret, ExposeSecret as _, SecretBox, SecretString, SerializableSecret};
use serde::{Deserialize, Serialize};
use serde_json::Number;

use crate::api::responses_read_http::config::ReadParams;

// the key a case or the Python child carries as text, kept out of Debug by the SecretBox around it
#[derive(Clone, Deserialize, Serialize)]
#[serde(transparent)]
pub struct LifecycleApiKey(String);
impl Zeroize for LifecycleApiKey {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}
impl CloneableSecret for LifecycleApiKey {}
impl SerializableSecret for LifecycleApiKey {}

// the kwargs both calls read: GenericLiteLLMParams' read fields plus the identity and metadata B247 logs
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct LifecycleParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretBox<LifecycleApiKey>>,
    pub ssl_verify: Option<bool>,
    pub litellm_call_id: Option<String>,
    pub metadata: Option<IndexMap<String, Json>>,
    pub litellm_metadata: Option<IndexMap<String, Json>>,
}

impl LifecycleParams {
    pub(crate) fn read_params(&self) -> ReadParams {
        ReadParams {
            api_base: self.api_base.clone(),
            api_key: self
                .api_key
                .as_ref()
                .map(|key| SecretString::from(key.expose_secret().0.clone())),
            ssl_verify: self.ssl_verify,
        }
    }
}

// get_responses declares timeout: float | httpx.Timeout | None; the handler is handed it and never applies it to the GET
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum LifecycleTimeout {
    Seconds(Number),
    Phases(LifecyclePhases),
}

// httpx.Timeout's four phases, each a number of seconds
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct LifecyclePhases {
    pub connect: Option<Number>,
    pub read: Option<Number>,
    pub write: Option<Number>,
    pub pool: Option<Number>,
}

// get_responses: the id, the provider the id may carry, the three extras and the handler's timeout
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponsesRetrieveCall {
    pub response_id: String,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub extra_query: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub extra_body: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub timeout: Option<LifecycleTimeout>,
    #[serde(flatten)]
    pub params: LifecycleParams,
    // the mode flag aget_responses hands the sync function; a call cannot name it
    #[serde(skip)]
    pub aget_responses: Option<Json>,
}

// alist_input_items: the five pagination arguments, whose absent limit and order are 20 and desc
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponsesInputItemsCall {
    pub response_id: String,
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub include: Option<Vec<String>>,
    #[serde(default = "default_limit")]
    pub limit: Option<i64>,
    #[serde(default = "default_order")]
    pub order: Option<InputItemsOrder>,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub timeout: Option<LifecycleTimeout>,
    #[serde(flatten)]
    pub params: LifecycleParams,
    #[serde(skip)]
    pub alist_input_items: Option<Json>,
}

const fn default_limit() -> Option<i64> {
    Some(20)
}

const fn default_order() -> Option<InputItemsOrder> {
    Some(InputItemsOrder::Desc)
}
