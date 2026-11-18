use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    AsyncInvokeStatusUrl(Call),
    StatusBodyPassthrough(Call),
    StatusHttpFailure(Call),
    StatusLogging(Call),
}

impl Input {
    pub(crate) fn call(&self) -> &Call {
        match self {
            Self::AsyncInvokeStatusUrl(call)
            | Self::StatusBodyPassthrough(call)
            | Self::StatusHttpFailure(call)
            | Self::StatusLogging(call) => call,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    pub(crate) invocation_arn: String,
    pub(crate) region: String,
    #[serde(default)]
    pub(crate) endpoint: Option<String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) credentials: Keys,
    #[serde(default = "default_status")]
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) logging: bool,
}

const fn default_status() -> u16 {
    200
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Keys {
    pub(crate) access_key_id: String,
    pub(crate) secret_access_key: String,
    #[serde(default)]
    pub(crate) session_token: Option<String>,
}
