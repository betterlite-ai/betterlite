use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::response_resources::request::{InputItemsOrder, ListInputItemsQuery};
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

#[derive(Debug, Clone, Copy)]
pub(super) enum LifecycleOperation {
    Retrieve,
    InputItems,
}

// what both reads carry: the id, the provider it may encode, and the kwargs the handler passes on
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ReadIdentity {
    pub response_id: String,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub timeout: Option<LifecycleTimeout>,
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(default)]
    pub api_key: Option<SecretBox<LifecycleApiKey>>,
    #[serde(default)]
    pub ssl_verify: Option<bool>,
    #[serde(default)]
    pub litellm_call_id: Option<String>,
    #[serde(default)]
    pub metadata: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub litellm_metadata: Option<IndexMap<String, Json>>,
}

impl ReadIdentity {
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

    // the read this call describes, with the operation's own extras
    pub(super) fn sdk_read(&self, extras: ReadExtras<'_>) -> SdkRead<'_> {
        SdkRead {
            identity: self,
            extras,
        }
    }
}

// one SDK read as the shared path needs it: the call's identity and the operation's own extras
pub(super) struct SdkRead<'call> {
    pub(super) identity: &'call ReadIdentity,
    pub(super) extras: ReadExtras<'call>,
}

// the retrieve body or the item query, and the flag that picks the async client, as Python's `is True` reads it
pub(super) enum ReadExtras<'call> {
    Retrieve {
        extra_body: Option<&'call IndexMap<String, Json>>,
        flag: Option<&'call Json>,
    },
    InputItems {
        query: ListInputItemsQuery,
        flag: Option<&'call Json>,
    },
}

impl ReadExtras<'_> {
    // the SDK operation the extras belong to
    pub(super) fn operation(&self) -> LifecycleOperation {
        match self {
            ReadExtras::Retrieve { .. } => LifecycleOperation::Retrieve,
            ReadExtras::InputItems { .. } => LifecycleOperation::InputItems,
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

// get_responses: the shared identity, its three extras and the mode flag aget_responses sets
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponsesRetrieveCall {
    #[serde(flatten)]
    pub identity: ReadIdentity,
    #[serde(default)]
    pub extra_query: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub extra_body: Option<IndexMap<String, Json>>,
    // the mode flag aget_responses hands the sync function; a call cannot name it
    #[serde(skip)]
    pub aget_responses: Option<Json>,
}

// alist_input_items: the shared identity and the five pagination arguments, whose absent limit and order are 20 and desc
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponsesInputItemsCall {
    #[serde(flatten)]
    pub identity: ReadIdentity,
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
    #[serde(skip)]
    pub alist_input_items: Option<Json>,
}

impl ResponsesInputItemsCall {
    // the five pagination arguments as the query the provider GET carries
    pub(super) fn query(&self) -> ListInputItemsQuery {
        ListInputItemsQuery {
            after: self.after.clone(),
            before: self.before.clone(),
            include: self.include.clone(),
            limit: self.limit,
            order: self.order,
        }
    }
}

const fn default_limit() -> Option<i64> {
    Some(20)
}

const fn default_order() -> Option<InputItemsOrder> {
    Some(InputItemsOrder::Desc)
}

mod tests;
