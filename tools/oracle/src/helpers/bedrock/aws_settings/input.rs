use std::collections::BTreeMap;

use betterlite_bedrock::aws_settings::credential_cache::Parameter;
use betterlite_config::sections::bedrock_settings::{AwsSettings, SessionTagValue};
use betterlite_config::tls::VerifySetting;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::value::RawValue;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", content = "params", rename_all = "snake_case")]
pub(crate) enum Input {
    StaticKeyPair(Pair),
    CanonicalTags(Tags),
    AssumeRole(AssumeRole),
    WebPolicy,
    Bearer(Bearer),
    PopAuth(BTreeMap<String, Box<RawValue>>),
    AuthSelector(Config),
    AuthError(AuthError),
    CacheKey(Keys),
    Cache(Cache),
    Local(Local),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Pair {
    pub(crate) s3_access_key_id: Option<SecretString>,
    pub(crate) s3_secret_access_key: Option<SecretString>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Tags {
    pub(crate) tags: Option<SessionTagValue>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AssumeRole {
    pub(crate) role_arn: String,
    pub(crate) session_name: String,
    pub(crate) external_id: Option<String>,
    pub(crate) tags: Option<SessionTagValue>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Bearer {
    pub(crate) api_key: Option<SecretString>,
    pub(crate) configured_token: Option<SecretString>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthError {
    pub(crate) status: u16,
    pub(crate) message: String,
    pub(crate) ssl_verify: Option<VerifySetting>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    #[serde(default)]
    #[serde(rename = "aws_access_key_id")]
    pub(crate) access_key_id: Option<SecretString>,
    #[serde(rename = "aws_secret_access_key")]
    pub(crate) secret_access_key: Option<SecretString>,
    #[serde(rename = "aws_session_token")]
    pub(crate) session_token: Option<SecretString>,
    #[serde(rename = "aws_region_name")]
    pub(crate) region_name: Option<String>,
    #[serde(rename = "aws_profile_name")]
    pub(crate) profile_name: Option<String>,
    #[serde(rename = "aws_role_name")]
    pub(crate) role_name: Option<String>,
    #[serde(rename = "aws_session_name")]
    pub(crate) session_name: Option<String>,
    #[serde(rename = "aws_web_identity_token")]
    pub(crate) web_identity_token: Option<SecretString>,
}

impl From<&Config> for AwsSettings {
    fn from(input: &Config) -> Self {
        Self {
            access_key_id: input.access_key_id.clone(),
            secret_access_key: input.secret_access_key.clone(),
            session_token: input.session_token.clone(),
            region_name: input.region_name.clone(),
            profile_name: input.profile_name.clone(),
            role_name: input.role_name.clone(),
            session_name: input.session_name.clone(),
            web_identity_token: input.web_identity_token.clone(),
            sts_endpoint: None,
            external_id: None,
            session_tags: None,
            role_arn: None,
            web_identity_token_file: None,
            bearer_token: None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Keys {
    pub(crate) left: BTreeMap<String, Parameter>,
    pub(crate) right: BTreeMap<String, Parameter>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Cache {
    pub(crate) parameters: BTreeMap<String, Parameter>,
    pub(crate) steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Step {
    pub(crate) now: u32,
    pub(crate) ttl: Option<u32>,
    pub(crate) access: Option<String>,
    #[serde(default)]
    pub(crate) fail: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Local {
    pub(crate) settings: Config,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) files: BTreeMap<String, String>,
    #[serde(rename = "fixture")]
    pub(crate) _fixture: [Option<String>; 3],
}
