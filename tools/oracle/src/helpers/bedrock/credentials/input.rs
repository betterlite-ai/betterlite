use std::collections::BTreeMap;

use betterlite_config::tls::VerifySetting;
use serde::Deserialize;
use serde_json::Value;

pub(super) const DEFAULT_NOW: i64 = 1_767_225_600;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    GetCredentials(Case),
    ResolveCredentials(Case),
    ResolveS3Credentials(Case),
    AuthWithAwsRole(Case),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Case {
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    #[serde(default)]
    pub now: Option<i64>,
    #[serde(default)]
    pub resolved_token: Option<String>,
    #[serde(default)]
    pub answers: Answers,
    pub steps: Vec<Step>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Step {
    pub params: BTreeMap<String, Value>,
    pub region: Option<String>,
    pub ssl_verify: Option<VerifySetting>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Answers {
    pub assume_role: Vec<Reply>,
    pub assume_role_with_web_identity: Vec<Reply>,
    pub get_caller_identity: Vec<Reply>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Reply {
    Credentials(ReplyCredentials),
    Identity {
        arn: String,
    },
    Error {
        status: u16,
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ReplyCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: String,
    pub expiration: String,
    pub packed_policy_size: Option<i32>,
}
