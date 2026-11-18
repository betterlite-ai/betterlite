use std::collections::BTreeMap;

use betterlite_openai::files::records::CreatePurpose;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    pub(super) steps: Vec<Step>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Action {
    Client,
    Create,
    Content,
    Retrieve,
    List,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Supplied {
    AzureOpenAi,
    AsyncAzureOpenAi,
    OpenAi,
    AsyncOpenAi,
}

// the builder a fixture client comes from; none is the missing-client branch LiteLLM checks after the call
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Builder {
    #[default]
    Default,
    None,
}

// the litellm_params the handler forwards; a key that is absent keeps the SDK client's own default
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Params {
    pub(super) azure_ad_token: Option<String>,
    pub(super) timeout: Option<f64>,
    pub(super) max_retries: Option<i64>,
}

// api_key, api_base, api_version, litellm_params build the client; timeout/max_retries only parse
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Values {
    pub(super) api_key: Option<String>,
    pub(super) api_base: Option<String>,
    pub(super) api_version: Option<String>,
    #[serde(default)]
    pub(super) litellm_params: Params,
    #[serde(default, rename = "timeout")]
    _timeout: f64,
    #[serde(default, rename = "max_retries")]
    _max_retries: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Step {
    pub(super) action: Action,
    pub(super) args: Values,
    #[serde(default)]
    pub(super) is_async: bool,
    #[serde(default)]
    pub(super) client: Option<Supplied>,
    #[serde(default)]
    pub(super) builder: Builder,
    #[serde(default)]
    pub(super) content: Option<String>,
    #[serde(default)]
    pub(super) purpose: Option<String>,
    #[serde(default)]
    pub(super) create_purpose: Option<CreatePurpose>,
    #[serde(default)]
    pub(super) expires_after: Option<i64>,
    #[serde(default)]
    pub(super) file_id: Option<String>,
    #[serde(default)]
    pub(super) reply: Option<Reply>,
}
