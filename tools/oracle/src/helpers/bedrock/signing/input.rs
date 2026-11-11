use betterlite_bedrock::signing::batch::{BatchData, BatchRequest as SignBatch};
use betterlite_bedrock::signing::service::{ServiceName, ServiceRequest as SignService};
use betterlite_bedrock::signing::target::RegionInputs;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::time::SystemTime;
#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub(crate) calls: Vec<Call>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Call {
    RequestHeaders(RequestHeaders),
    ServiceRequest(ServiceRequest),
    JsonPost(JsonPost),
    RequestTarget(RequestTarget),
    BatchRequest(BatchRequest),
    OffLoop(OffLoop),
}
#[derive(Debug, Deserialize)]
pub(crate) struct CredentialInput {
    pub(crate) access_key_id: SecretString,
    pub(crate) secret_access_key: SecretString,
    pub(crate) session_token: Option<SecretString>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DataInput {
    Text(String),
    Base64(String),
}
#[derive(Debug, Deserialize)]
pub(crate) struct RequestHeaders {
    pub(crate) credentials: Option<CredentialInput>,
    pub(crate) region: String,
    pub(crate) extra_headers: BTreeMap<String, String>,
    pub(crate) endpoint: String,
    pub(crate) data: DataInput,
    pub(crate) headers: BTreeMap<String, Option<String>>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) configured_bearer_token: Option<SecretString>,
    #[serde(default = "default_supports_bearer_token")]
    pub(crate) supports_bearer_token: bool,
    pub(crate) now: i64,
}
#[derive(Debug, Deserialize)]
pub(crate) struct ServiceRequest {
    pub(crate) service_name: ServiceName,
    pub(crate) headers: BTreeMap<String, Option<String>>,
    pub(crate) optional_params: BTreeMap<String, Box<RawValue>>,
    pub(crate) request_data: IndexMap<String, Json>,
    pub(crate) api_base: String,
    pub(crate) model: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) configured_bearer_token: Option<SecretString>,
    pub(crate) now: i64,
    pub(crate) credentials: CredentialInput,
}
#[derive(Debug, Deserialize)]
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
pub(crate) struct RequestTarget {
    pub(crate) optional_params: BTreeMap<String, Box<RawValue>>,
    pub(crate) model: Option<String>,
    pub(crate) bearer_token: Option<SecretString>,
    pub(crate) credentials: CredentialInput,
}
#[derive(Debug, Deserialize)]
pub(crate) struct BatchRequest {
    pub(crate) service_name: String,
    pub(crate) data: BatchDataInput,
    pub(crate) endpoint_url: String,
    pub(crate) optional_params: BTreeMap<String, Box<RawValue>>,
    pub(crate) method: Option<String>,
    pub(crate) now: i64,
    pub(crate) credentials: CredentialInput,
}
impl ServiceRequest {
    pub(crate) fn request<'source>(
        &'source self,
        headers: &'source [(String, Option<String>)],
        now: SystemTime,
    ) -> SignService<'source> {
        SignService {
            service: self.service_name,
            headers,
            optional_params: &self.optional_params,
            request_data: &self.request_data,
            api_base: &self.api_base,
            model: self.model.as_deref(),
            api_key: self.api_key.as_ref(),
            configured_bearer: self.configured_bearer_token.as_ref(),
            region_inputs: RegionInputs::default(),
            now,
        }
    }
}
impl BatchRequest {
    pub(crate) fn request(&self, now: SystemTime) -> SignBatch<'_> {
        let data = match &self.data {
            BatchDataInput::Json(object) => BatchData::Json(object),
            BatchDataInput::Text(text) => BatchData::Text(text),
        };
        SignBatch {
            service_name: &self.service_name,
            data,
            endpoint: &self.endpoint_url,
            optional_params: &self.optional_params,
            method: self.method.as_deref().unwrap_or("POST"),
            region_inputs: RegionInputs::default(),
            now,
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum BatchDataInput {
    Json(IndexMap<String, Json>),
    Text(String),
}
#[derive(Debug, Deserialize)]
pub(crate) struct OffLoop {
    pub(crate) context: String,
    pub(crate) value: String,
    pub(crate) failure: String,
}
const fn default_supports_bearer_token() -> bool {
    true
}
