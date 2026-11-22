use std::collections::BTreeMap;

use betterlite_openai::files::records::CreatePurpose;
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    ObjectName,
    UrlConversion,
    Sign,
    FileResponse,
    Destination,
    Contract,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) operation: Operation,
    #[serde(default)]
    pub(crate) files: Vec<ObjectFile>,
    #[serde(default)]
    pub(crate) urls: Vec<String>,
    #[serde(default)]
    pub(crate) signings: Vec<Signing>,
    #[serde(default)]
    pub(crate) responses: Vec<Response>,
    #[serde(default)]
    pub(crate) errors: Vec<ErrorCall>,
    #[serde(default)]
    pub(crate) uploads: Vec<Upload>,
    #[serde(default)]
    pub(crate) handoffs: Vec<Handoff>,
    #[serde(default)]
    pub(crate) optional_params: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ObjectFile {
    pub(crate) content: String,
    #[serde(default)]
    pub(crate) content_hex: Option<String>,
    pub(crate) purpose: CreatePurpose,
    pub(crate) filename: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Signing {
    pub(crate) content: String,
    pub(crate) api_base: String,
    pub(crate) optional_params: Map<String, Value>,
    pub(crate) encryption_key_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Response {
    pub(crate) litellm_params: Map<String, Value>,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) now: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorCall {
    pub(crate) status_code: u16,
    pub(crate) message: String,
    pub(crate) headers: Vec<(String, String)>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UploadFile {
    pub(crate) filename: Option<String>,
    pub(crate) content: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Upload {
    pub(crate) model: String,
    pub(crate) litellm_params: Map<String, Value>,
    pub(crate) optional_params: Map<String, Value>,
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) file: Option<UploadFile>,
    #[serde(default)]
    pub(crate) purpose: Option<CreatePurpose>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Handoff {
    pub(crate) headers: Map<String, Value>,
    pub(crate) litellm_params: Map<String, Value>,
}
