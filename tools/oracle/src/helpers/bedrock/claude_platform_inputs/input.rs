use std::collections::BTreeMap;

use betterlite_bedrock::model_routes::ParamMap;
use betterlite_config::sections::claude_platform_inputs::UnsupportedParams;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::value::RawValue;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    WorkspaceAliasPrecedence { cases: Vec<WorkspaceCase> },
    ExplicitRegionRequired { cases: Vec<RegionCase> },
    ApiBaseBeforeRegion { cases: Vec<UrlCase> },
    PlatformRoutePrefix { models: Vec<String> },
    UnsupportedOverride { calls: Vec<OverrideCall> },
    FilteredBodyFields { calls: Vec<BodyCall> },
    PlatformSigning { calls: Vec<SigningCall> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkspaceCase {
    #[serde(default)]
    pub(crate) optional_params: ParamMap,
    #[serde(default)]
    pub(crate) deployment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) litellm_settings: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegionCase {
    #[serde(default)]
    pub(crate) optional_params: ParamMap,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlCase {
    pub(crate) api_base: Option<String>,
    pub(crate) litellm_api_base: Option<String>,
    #[serde(default)]
    pub(crate) optional_params: ParamMap,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OverrideCall {
    pub(crate) deployment: Option<UnsupportedParams>,
    pub(crate) optional: Option<UnsupportedParams>,
    #[serde(default = "default_true")]
    pub(crate) log_invalid: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BodyCall {
    #[serde(default)]
    pub(crate) params: ParamMap,
    pub(crate) unsupported: Option<Vec<String>>,
    #[serde(default = "default_true")]
    pub(crate) log_dropped: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SigningCall {
    Sign(Box<SignCall>),
    ErrorClass(ErrorClassCall),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SignCall {
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, Option<String>>,
    #[serde(default)]
    pub(crate) optional_params: BTreeMap<String, Box<RawValue>>,
    #[serde(default)]
    pub(crate) request_data: IndexMap<String, Json>,
    pub(crate) api_base: String,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) model: Option<String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) credentials: CredentialInput,
    pub(crate) now: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorClassCall {
    pub(crate) message: String,
    pub(crate) status_code: u16,
    #[serde(default)]
    pub(crate) headers: Vec<(String, String)>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialInput {
    pub(crate) access_key_id: SecretString,
    pub(crate) secret_access_key: SecretString,
    pub(crate) session_token: Option<SecretString>,
}

const fn default_true() -> bool {
    true
}
