use std::collections::BTreeMap;

use secrecy::SecretString;
use serde::Deserialize;

use crate::arguments::json::Json;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMethod {
    Basic,
    // LiteLLM sends the bearer token for every method that is not "basic"
    #[serde(other)]
    Token,
}

#[derive(Debug, Deserialize)]
pub struct BitBucketConfig {
    pub workspace: Option<String>,
    pub repository: Option<String>,
    pub access_token: Option<SecretString>,
    pub branch: Option<String>,
    // LiteLLM reads the API base from the key "", not from the documented "base_url"
    #[serde(rename = "")]
    pub base_url: Option<String>,
    pub auth_method: Option<AuthMethod>,
    pub username: Option<String>,
    #[serde(flatten)]
    pub unrecognized: BTreeMap<String, Json>,
}
