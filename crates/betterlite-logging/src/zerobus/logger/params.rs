use std::num::{NonZeroU64, NonZeroUsize};

use indexmap::IndexMap;
use secrecy::SecretString;
use thiserror::Error;

use crate::zerobus::client::init_params::ZerobusInitParams;

pub const TURN_OFF_MESSAGE_LOGGING: &str = "turn_off_message_logging";
pub const BATCH_SIZE: &str = "batch_size";
pub const FLUSH_INTERVAL: &str = "flush_interval";
pub const WORKSPACE_URL: &str = "workspace_url";
pub const SERVER_ENDPOINT: &str = "server_endpoint";
pub const CLIENT_ID: &str = "client_id";
pub const CLIENT_SECRET: &str = "client_secret";
pub const TABLE_NAME: &str = "table_name";

#[derive(Debug, Error)]
pub enum ParamsError {
    #[error("callback_settings.zerobus.{field} must be a number")]
    Number { field: String },
    #[error("callback_settings.zerobus.{field} must be a bool")]
    Bool { field: String },
    #[error("callback_settings.zerobus.{field} is not a zerobus setting")]
    Unknown { field: String },
}

pub fn configured_params(
    variables: Option<&IndexMap<String, String>>,
) -> Result<ZerobusInitParams, ParamsError> {
    let mut params = ZerobusInitParams::default();
    let Some(variables) = variables else {
        return Ok(params);
    };
    for (key, value) in variables {
        apply(&mut params, key, value)?;
    }
    Ok(params)
}

fn apply(params: &mut ZerobusInitParams, key: &str, value: &str) -> Result<(), ParamsError> {
    match key {
        WORKSPACE_URL => params.workspace_url = Some(value.to_owned()),
        SERVER_ENDPOINT => params.server_endpoint = Some(value.to_owned()),
        CLIENT_ID => params.client_id = Some(value.to_owned()),
        CLIENT_SECRET => params.client_secret = Some(SecretString::from(value)),
        TABLE_NAME => params.table_name = Some(value.to_owned()),
        BATCH_SIZE => params.batch_size = batch_size(key, value)?,
        FLUSH_INTERVAL => params.flush_interval = flush_interval(key, value)?,
        TURN_OFF_MESSAGE_LOGGING => params.turn_off_message_logging = flag(key, value)?,
        _ => {
            return Err(ParamsError::Unknown {
                field: key.to_owned(),
            });
        }
    }
    Ok(())
}

fn batch_size(field: &str, value: &str) -> Result<NonZeroUsize, ParamsError> {
    let parsed = betterlite_env::number::<NonZeroUsize>(field, value)
        .map_err(|_source| ParamsError::Number {
            field: field.to_owned(),
        })?
        .unwrap_or(ZerobusInitParams::default().batch_size);
    Ok(parsed)
}

fn flush_interval(field: &str, value: &str) -> Result<NonZeroU64, ParamsError> {
    let parsed = betterlite_env::number::<NonZeroU64>(field, value)
        .map_err(|_source| ParamsError::Number {
            field: field.to_owned(),
        })?
        .unwrap_or(ZerobusInitParams::default().flush_interval);
    Ok(parsed)
}

fn flag(field: &str, value: &str) -> Result<bool, ParamsError> {
    let parsed = betterlite_env::flag(field, value)
        .map_err(|_source| ParamsError::Bool {
            field: field.to_owned(),
        })?
        .unwrap_or(false);
    Ok(parsed)
}
