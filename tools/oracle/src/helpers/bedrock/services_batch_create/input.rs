use std::collections::BTreeMap;

use secrecy::SecretString;
use serde::Deserialize;
use serde_json::Value;
use serde_json::value::RawValue;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    BatchRoleRequired,
    OutputBucketFallback,
    TagsValidation,
    // serde's snake_case turns the digits into input24h_window, but the wire name keeps the underscore
    #[serde(rename = "input_24h_window")]
    Input24hWindow,
    CreateStatusMapping,
    MetadataSanitization,
    GeneratedJobName,
    OwnerAndKms,
    CreateSignedRequest,
    CreateReplyDefaults,
    TitanBatchTokenUsage,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) operation: Operation,
    #[serde(default)]
    pub(crate) scenarios: Vec<Scenario>,
    #[serde(default)]
    pub(crate) values: Vec<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scenario {
    #[serde(default)]
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) input_file_id: Option<String>,
    #[serde(default)]
    pub(crate) completion_window: Option<String>,
    #[serde(default)]
    pub(crate) litellm_params: BTreeMap<String, Box<RawValue>>,
    #[serde(default)]
    pub(crate) optional_params: BTreeMap<String, Box<RawValue>>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) original_batch_request: Option<Value>,
    #[serde(default)]
    pub(crate) response_body: String,
    #[serde(default)]
    pub(crate) clock: Vec<i64>,
    #[serde(default)]
    pub(crate) job_prefix: Option<String>,
    #[serde(default = "default_uuid")]
    pub(crate) uuid: String,
    #[serde(default)]
    pub(crate) credentials: Option<CredentialsInput>,
    #[serde(default)]
    pub(crate) now: i64,
}

pub(crate) const DEFAULT_UUID: &str = "00000000-0000-4000-8000-000000000000";

fn default_uuid() -> String {
    DEFAULT_UUID.to_owned()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialsInput {
    pub(crate) access_key_id: SecretString,
    pub(crate) secret_access_key: SecretString,
    #[serde(default)]
    pub(crate) session_token: Option<SecretString>,
}
