use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_provider::api::vector_store_contracts::config::Params;
use secrecy::SecretString;

use super::build_valkey_url;
use super::error::ValkeyError;
use super::query::{DEFAULT_EMBEDDING_FIELD, DEFAULT_TEXT_FIELD};

pub const DEFAULT_PORT: u16 = 6379;

#[derive(Debug)]
pub struct ValkeyParams {
    pub embedding_model: Option<String>,
    pub embedding_config: Params,
    pub host: Option<String>,
    pub port: u16,
    pub password: Option<SecretString>,
    pub ssl: bool,
    pub text_field: String,
    pub embedding_field: String,
}

impl ValkeyParams {
    pub fn read(params: &Params) -> Result<Self, ValkeyError> {
        Ok(Self {
            embedding_model: text(params, "litellm_embedding_model")?,
            embedding_config: embedding_config(params)?,
            host: text(params, "valkey_host")?,
            port: port(params)?,
            password: text(params, "valkey_password")?.map(SecretString::from),
            ssl: flag(params, "valkey_ssl")?.unwrap_or(false),
            text_field: text(params, "valkey_text_field")?
                .unwrap_or_else(|| DEFAULT_TEXT_FIELD.to_owned()),
            embedding_field: text(params, "valkey_embedding_field")?
                .unwrap_or_else(|| DEFAULT_EMBEDDING_FIELD.to_owned()),
        })
    }

    pub fn connection_url(&self) -> Result<String, ValkeyError> {
        self.host.as_deref().map_or_else(
            || Err(ValkeyError::HostRequired),
            |host| {
                Ok(build_valkey_url(
                    host,
                    self.port,
                    self.password.as_ref(),
                    self.ssl,
                ))
            },
        )
    }
}

fn text(params: &Params, field: &'static str) -> Result<Option<String>, ValkeyError> {
    match params.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(value)) => Ok(Some(value.clone())),
        Some(_) => Err(ValkeyError::FieldType {
            field,
            expected: "a string",
        }),
    }
}

fn flag(params: &Params, field: &'static str) -> Result<Option<bool>, ValkeyError> {
    match params.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(ValkeyError::FieldType {
            field,
            expected: "a boolean",
        }),
    }
}

fn port(params: &Params) -> Result<u16, ValkeyError> {
    match params.get("valkey_port") {
        None | Some(Json::Null) => Ok(DEFAULT_PORT),
        Some(Json::Number(number)) => {
            let value = number.as_i64().ok_or(ValkeyError::FieldType {
                field: "valkey_port",
                expected: "an integer",
            })?;
            u16::try_from(value).map_err(|_source| ValkeyError::PortRange { value })
        }
        Some(_) => Err(ValkeyError::FieldType {
            field: "valkey_port",
            expected: "an integer",
        }),
    }
}

fn embedding_config(params: &Params) -> Result<Params, ValkeyError> {
    match params.get("litellm_embedding_config") {
        None | Some(Json::Null) => Ok(BTreeMap::new()),
        Some(Json::Object(entries)) => Ok(entries
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()),
        Some(_) => Err(ValkeyError::FieldType {
            field: "litellm_embedding_config",
            expected: "an object",
        }),
    }
}
