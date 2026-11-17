use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use super::{ContractError, decode, require_object};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CoordinationRedisNode {
    pub host: String,
    pub port: i64,
}

impl CoordinationRedisNode {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let fields = require_object(value)?;
        let request: Self = decode(&fields)?;
        Ok(request)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum SentinelValue {
    Text(String),
    Integer(i64),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum BoolOrText {
    Bool(bool),
    Text(String),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CoordinationRedisParams {
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub port: Option<i64>,
    #[serde(
        default,
        serialize_with = "serde_with::As::<Option<serde_with::FromIntoRef<crate::key_fields::values::ExposedSecret>>>::serialize"
    )]
    pub password: Option<SecretString>,
    #[serde(default)]
    pub username: Option<String>,
    // LiteLLM's field description says the url embeds the password (redis://:pass@host), so Debug must redact it.
    #[serde(
        default,
        serialize_with = "serde_with::As::<Option<serde_with::FromIntoRef<crate::key_fields::values::ExposedSecret>>>::serialize"
    )]
    pub url: Option<SecretString>,
    #[serde(default)]
    pub ssl: Option<bool>,
    #[serde(default)]
    pub startup_nodes: Option<Vec<CoordinationRedisNode>>,
    #[serde(default)]
    pub sentinel_nodes: Option<Vec<Vec<SentinelValue>>>,
    #[serde(
        default,
        serialize_with = "serde_with::As::<Option<serde_with::FromIntoRef<crate::key_fields::values::ExposedSecret>>>::serialize"
    )]
    pub sentinel_password: Option<SecretString>,
    #[serde(default)]
    pub service_name: Option<String>,
    #[serde(default)]
    pub aws_iam_auth: Option<BoolOrText>,
    #[serde(default)]
    pub aws_iam_user_name: Option<String>,
    #[serde(default)]
    pub aws_iam_cache_name: Option<String>,
    #[serde(default)]
    pub aws_iam_region: Option<String>,
    #[serde(default)]
    pub aws_iam_serverless: Option<BoolOrText>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, JsonValue>,
}

impl CoordinationRedisParams {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let fields = require_object(value)?;
        let request: Self = decode(&fields)?;
        Ok(request)
    }

    // An empty string or an empty list still selects a connection, and a port on its own selects nothing.
    #[must_use]
    pub const fn has_connection_target(&self) -> bool {
        self.host.is_some()
            || self.url.is_some()
            || self.startup_nodes.is_some()
            || self.sentinel_nodes.is_some()
    }
}

mod tests;
