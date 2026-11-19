use std::collections::BTreeMap;

use betterlite_bedrock::aws_endpoints::runtime::BedrockEndpointType;
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Deserialize)]
pub(crate) enum Input {
    #[serde(rename = "standalone_mantle_base")]
    Base {
        #[serde(default)]
        model_cost: ParamMap,
        cases: Vec<BaseCase>,
    },
    #[serde(rename = "standalone_mantle_auth")]
    Auth { cases: Vec<SignCase> },
    #[serde(rename = "chat_provider_prefix")]
    Chat {
        #[serde(default)]
        catalog: Catalog,
        cases: Vec<ChatCase>,
    },
    #[serde(rename = "messages_auth_and_url")]
    Messages { cases: Vec<MessageCase> },
    #[serde(rename = "mantle_passthrough_path")]
    Passthrough { cases: Vec<PassthroughCase> },
    #[serde(rename = "mantle_auth_failure")]
    Failure { cases: Vec<FailureCase> },
    #[serde(rename = "mantle_passthrough_reply_shape")]
    ReplyShape { cases: Vec<ReplyCase> },
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Catalog {
    #[serde(default)]
    pub(super) chat_models: Vec<String>,
    #[serde(default)]
    pub(super) text_models: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BaseCase {
    #[serde(default)]
    pub(super) model: Option<String>,
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) aws_region_name: Option<String>,
    #[serde(default)]
    pub(super) inject_signer: bool,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Keys {
    pub(super) access_key_id: String,
    pub(super) secret_access_key: String,
    #[serde(default)]
    pub(super) session_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SignCase {
    pub(super) api_base: String,
    #[serde(default)]
    pub(super) headers: Vec<(String, Option<String>)>,
    #[serde(default)]
    pub(super) optional_params: Map<String, Value>,
    #[serde(default)]
    pub(super) request_data: IndexMap<String, Json>,
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) model: Option<String>,
    #[serde(default)]
    pub(super) stream: Option<bool>,
    #[serde(default)]
    pub(super) fake_stream: Option<bool>,
    pub(super) credentials: Keys,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    pub(super) now: i64,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Reasoning {
    Supported,
    #[default]
    Unsupported,
    Panic,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ErrorProbe {
    pub(super) message: String,
    pub(super) status_code: u16,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ChatCase {
    pub(super) model: String,
    #[serde(default)]
    pub(super) project_id: Option<String>,
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) reasoning: Reasoning,
    pub(super) error: ErrorProbe,
    #[serde(default)]
    pub(super) json_mode: Option<bool>,
    #[serde(default)]
    pub(super) sync_stream: bool,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MessageCase {
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) aws_region_name: Option<String>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PassthroughCase {
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) aws_region_name: Option<String>,
    #[serde(default)]
    pub(super) aws_bedrock_runtime_endpoint: Option<String>,
    #[serde(default)]
    pub(super) endpoint_type: BedrockEndpointType,
    #[serde(default)]
    pub(super) api_key: Option<Value>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum FailureKind {
    NoCredentials,
    PartialCredentials,
    ProfileNotFound,
    CredentialRetrieval,
    Region,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FailureCase {
    pub(super) kind: FailureKind,
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) aws_region_name: Option<String>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReplyCase {
    pub(super) custom_llm_provider: String,
    pub(super) endpoint: String,
}
