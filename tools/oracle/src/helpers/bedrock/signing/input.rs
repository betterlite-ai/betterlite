use std::collections::BTreeMap;

use betterlite_bedrock::signing::service::ServiceName;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::value::RawValue;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Call {
    RequestHeaders(RequestHeaders),
    FilterHeaders(FilterHeaders),
    ServiceRequest(ServiceRequest),
    JsonPost(JsonPost),
    RequestTarget(RequestTarget),
    BatchRequest(BatchRequest),
    OffLoop(OffLoop),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialInput {
    pub(crate) access_key_id: String,
    pub(crate) secret_access_key: String,
    pub(crate) session_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DataInput {
    Text(String),
    Base64(String),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestHeaders {
    pub(crate) credentials: Option<CredentialInput>,
    pub(crate) region: String,
    pub(crate) extra_headers: BTreeMap<String, String>,
    pub(crate) endpoint: String,
    pub(crate) data: DataInput,
    pub(crate) headers: BTreeMap<String, Option<String>>,
    pub(crate) api_key: Option<String>,
    pub(crate) configured_bearer_token: Option<String>,
    #[serde(default = "default_supports_bearer_token")]
    pub(crate) supports_bearer_token: bool,
    pub(crate) now: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FilterHeaders {
    pub(crate) headers: BTreeMap<String, Option<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServiceRequest {
    pub(crate) service_name: ServiceName,
    pub(crate) headers: BTreeMap<String, Option<String>>,
    pub(crate) optional_params: BTreeMap<String, Box<RawValue>>,
    pub(crate) request_data: IndexMap<String, Json>,
    pub(crate) api_base: String,
    pub(crate) model: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) configured_bearer_token: Option<String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) now: i64,
    pub(crate) credentials: CredentialInput,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JsonPost {
    pub(crate) credentials: CredentialInput,
    pub(crate) service_name: String,
    pub(crate) region: Option<String>,
    pub(crate) url: String,
    pub(crate) body: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) now: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestTarget {
    pub(crate) optional_params: BTreeMap<String, Box<RawValue>>,
    pub(crate) model: Option<String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) bearer_token: Option<String>,
    pub(crate) credentials: CredentialInput,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BatchRequest {
    pub(crate) service_name: String,
    pub(crate) data: BatchDataInput,
    pub(crate) endpoint_url: String,
    pub(crate) optional_params: BTreeMap<String, Box<RawValue>>,
    pub(crate) method: Option<String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) now: i64,
    pub(crate) credentials: CredentialInput,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum BatchDataInput {
    Json(IndexMap<String, Json>),
    Text(String),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OffLoop {
    pub(crate) context: String,
    pub(crate) value: String,
    pub(crate) failure: String,
}

const fn default_supports_bearer_token() -> bool {
    true
}
