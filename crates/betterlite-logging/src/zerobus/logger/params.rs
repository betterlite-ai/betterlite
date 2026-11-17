use std::str::FromStr;

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
    let Some(configured) = variables else {
        return Ok(params);
    };
    for (key, value) in configured {
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
        BATCH_SIZE => {
            params.batch_size = number(key, value, ZerobusInitParams::default().batch_size)?;
        }
        FLUSH_INTERVAL => {
            params.flush_interval =
                number(key, value, ZerobusInitParams::default().flush_interval)?;
        }
        TURN_OFF_MESSAGE_LOGGING => {
            params.turn_off_message_logging = betterlite_env::flag(key, value)
                .map_err(|_source| ParamsError::Bool {
                    field: key.to_owned(),
                })?
                .unwrap_or(false);
        }
        _ => {
            return Err(ParamsError::Unknown {
                field: key.to_owned(),
            });
        }
    }
    Ok(())
}

fn number<T>(field: &str, value: &str, fallback: T) -> Result<T, ParamsError>
where
    T: FromStr,
{
    let parsed = betterlite_env::number::<T>(field, value)
        .map_err(|_source| ParamsError::Number {
            field: field.to_owned(),
        })?
        .unwrap_or(fallback);
    Ok(parsed)
}
