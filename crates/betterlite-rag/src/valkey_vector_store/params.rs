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
        let embedding_model = text(params, "litellm_embedding_model")?;
        let embedding_config = embedding_config(params)?;
        let host = text(params, "valkey_host")?;
        let port = port(params)?;
        let password = text(params, "valkey_password")?.map(SecretString::from);
        let text_field =
            text(params, "valkey_text_field")?.unwrap_or_else(|| DEFAULT_TEXT_FIELD.to_owned());
        let embedding_field = text(params, "valkey_embedding_field")?
            .unwrap_or_else(|| DEFAULT_EMBEDDING_FIELD.to_owned());
        let ssl = match params
            .get("valkey_ssl")
            .filter(|value| !matches!(value, Json::Null))
        {
            None => false,
            Some(Json::Bool(flag)) => *flag,
            Some(
                Json::Null | Json::Text(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            ) => {
                return Err(ValkeyError::FieldType {
                    field: "valkey_ssl",
                    expected: "a boolean",
                });
            }
        };
        Ok(Self {
            embedding_model,
            embedding_config,
            host,
            port,
            password,
            ssl,
            text_field,
            embedding_field,
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
    match params
        .get(field)
        .filter(|value| !matches!(value, Json::Null))
    {
        None => Ok(None),
        Some(Json::Text(value)) => Ok(Some(value.clone())),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ValkeyError::FieldType {
                field,
                expected: "a string",
            })
        }
    }
}

fn port(params: &Params) -> Result<u16, ValkeyError> {
    let Some(value) = params
        .get("valkey_port")
        .filter(|value| !matches!(value, Json::Null))
    else {
        return Ok(DEFAULT_PORT);
    };
    let Json::Number(number) = value else {
        return Err(ValkeyError::FieldType {
            field: "valkey_port",
            expected: "an integer",
        });
    };
    let port_number = number.as_i64().ok_or(ValkeyError::FieldType {
        field: "valkey_port",
        expected: "an integer",
    })?;
    u16::try_from(port_number).map_err(|_source| ValkeyError::PortRange { value: port_number })
}

fn embedding_config(params: &Params) -> Result<Params, ValkeyError> {
    let Some(value) = params
        .get("litellm_embedding_config")
        .filter(|value| !matches!(value, Json::Null))
    else {
        return Ok(BTreeMap::new());
    };
    match value {
        Json::Object(entries) => Ok(entries
            .iter()
            .map(|(name, entry)| (name.clone(), entry.clone()))
            .collect()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(ValkeyError::FieldType {
                field: "litellm_embedding_config",
                expected: "an object",
            })
        }
    }
}
