use std::collections::BTreeMap;

use betterlite_bedrock::aws_endpoints::provider::BedrockInvokeProvider;
use betterlite_bedrock::invoke_primitives::config::Params;
use betterlite_bedrock::invoke_primitives::prompt::CustomPromptEntry;
use betterlite_models::decisions::Json;
use betterlite_prompts::builtin::Message;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::value::RawValue;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    BaseOptions {
        non_default: Params,
        optional: Params,
        config: Params,
    },
    ProviderFamilySelection {
        models: Vec<String>,
    },
    InvokeModelUrl {
        cases: Vec<UrlCase>,
    },
    GuardrailHeaders {
        values: Vec<Box<RawValue>>,
    },
    GuardrailMetadataMerge {
        merges: Vec<MergeCase>,
        error: ErrorCase,
    },
    SigningForwarding(SigningCase),
    PromptTemplateSelection {
        cases: Vec<PromptCase>,
    },
    LlamaLocalTemplate {
        local_assets: bool,
        cases: Vec<PromptCase>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlCase {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) stream: Option<bool>,
    #[serde(default)]
    pub(crate) optional_params: Params,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MergeCase {
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) optional_params: Params,
    #[serde(default)]
    pub(crate) identity: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) spend_logs_metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) policy_fields: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorCase {
    pub(crate) message: String,
    pub(crate) status_code: u16,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SigningCase {
    pub(crate) credentials: CredentialInput,
    pub(crate) headers: BTreeMap<String, Option<String>>,
    #[serde(default)]
    pub(crate) optional_params: Params,
    #[serde(default)]
    pub(crate) request_data: IndexMap<String, Json>,
    pub(crate) api_base: String,
    #[serde(default)]
    pub(crate) model: Option<String>,
    #[serde(default)]
    pub(crate) api_key: Option<SecretString>,
    #[serde(default)]
    pub(crate) configured_bearer_token: Option<SecretString>,
    #[serde(default)]
    pub(crate) stream: Option<bool>,
    #[serde(default)]
    pub(crate) fake_stream: Option<bool>,
    pub(crate) now: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialInput {
    pub(crate) access_key_id: SecretString,
    pub(crate) secret_access_key: SecretString,
    #[serde(default)]
    pub(crate) session_token: Option<SecretString>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PromptCase {
    pub(crate) model: String,
    pub(crate) messages: Vec<Message>,
    #[serde(default)]
    pub(crate) provider: Option<BedrockInvokeProvider>,
    #[serde(default)]
    pub(crate) custom_prompt_dict: BTreeMap<String, CustomPromptEntry>,
}
