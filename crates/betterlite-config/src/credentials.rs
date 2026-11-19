use std::collections::BTreeMap;
use std::fmt;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use subtle::ConstantTimeEq as _;
use thiserror::Error;

#[derive(Clone)]
pub struct SecretText(SecretString);

impl PartialEq for SecretText {
    fn eq(&self, other: &Self) -> bool {
        bool::from(
            self.0
                .expose_secret()
                .as_bytes()
                .ct_eq(other.0.expose_secret().as_bytes()),
        )
    }
}

impl Eq for SecretText {}

// Values stay secret in memory; only the encryption and masking boundaries open them.
#[derive(Clone, PartialEq, Eq)]
pub enum CredentialValue {
    Text(SecretText),
    Null,
    Bool(bool),
    Number(serde_json::Number),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

impl CredentialValue {
    #[must_use]
    pub fn expose_json(&self) -> JsonData {
        match self {
            Self::Text(text) => JsonData::String(text.0.expose_secret().to_owned()),
            Self::Null => JsonData::Null,
            Self::Bool(value) => JsonData::Bool(*value),
            Self::Number(value) => JsonData::Number(value.clone()),
            Self::Array(values) => JsonData::Array(values.iter().map(Self::expose_json).collect()),
            Self::Object(fields) => JsonData::Object(
                fields
                    .iter()
                    .map(|(key, value)| (key.clone(), value.expose_json()))
                    .collect(),
            ),
        }
    }
}

impl fmt::Debug for CredentialValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // both shapes can carry a secret, so neither prints its contents
        f.write_str("CredentialValue([REDACTED])")
    }
}

impl From<JsonData> for CredentialValue {
    fn from(value: JsonData) -> Self {
        match value {
            JsonData::String(text) => Self::Text(SecretText(SecretString::from(text))),
            JsonData::Null => Self::Null,
            JsonData::Bool(flag) => Self::Bool(flag),
            JsonData::Number(number) => Self::Number(number),
            JsonData::Array(values) => Self::Array(values.into_iter().map(Self::from).collect()),
            JsonData::Object(fields) => Self::Object(
                fields
                    .into_iter()
                    .map(|(key, item)| (key, Self::from(item)))
                    .collect(),
            ),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(try_from = "JsonData")]
pub struct CredentialValues(pub BTreeMap<String, CredentialValue>);

#[derive(Debug, Error)]
pub enum CredentialValuesError {
    #[error("credential_values must be a mapping of names to values")]
    NotAnObject,
}

impl TryFrom<JsonData> for CredentialValues {
    type Error = CredentialValuesError;

    fn try_from(value: JsonData) -> Result<Self, Self::Error> {
        let JsonData::Object(fields) = value else {
            return Err(CredentialValuesError::NotAnObject);
        };
        Ok(Self::from(Payload(fields)))
    }
}

impl From<Payload> for CredentialValues {
    fn from(payload: Payload) -> Self {
        let mut values = BTreeMap::new();
        for (name, value) in payload.0 {
            values.insert(name, CredentialValue::from(value));
        }
        Self(values)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CredentialItem {
    pub credential_name: String,
    pub credential_info: Payload,
    pub credential_values: CredentialValues,
}

mod tests;
