use std::io::Write;

use betterlite_config::sections::settings::JsonValue;
use http::Method;
use rust_decimal::Decimal;
use serde_json::Number;

use super::api::ApiError;
use super::client::KeysClient;
use super::output;
use super::wire::KeyGenerateBody;
use super::{GenerateKey, KeysError, KeysPorts, json_string_list};

const GENERATE_PATH: &str = "/key/generate";

pub(super) fn generate_key(options: &GenerateKey, ports: KeysPorts<'_>) -> Result<(), KeysError> {
    let KeysPorts {
        context,
        client,
        redactor,
        stdout,
        stderr,
    } = ports;
    let body = generate_body(options)?;
    let api = KeysClient {
        transport: client,
        base_url: context.base_url.trim_end_matches('/'),
        api_key: context.api_key.as_ref(),
        redactor,
    };
    match api.send(Method::POST, GENERATE_PATH, &[], Some(body)) {
        Ok(reply) => output::write_pretty(&reply.value, stdout).map_err(KeysError::Json),
        Err(ApiError::Http { status, body, .. }) => {
            output::print_http_error(status, &body, stdout, stderr).map_err(KeysError::Json)?;
            Err(KeysError::Aborted)
        }
        Err(error) => Err(KeysError::Client(error)),
    }
}

fn generate_body(options: &GenerateKey) -> Result<Vec<u8>, KeysError> {
    let body = KeyGenerateBody {
        models: json_string_list(options.models.as_deref()),
        aliases: parse_aliases(options.aliases.as_deref())?,
        spend: parse_spend(options.spend)?,
        duration: options.duration.clone(),
        key_alias: options.key_alias.clone(),
        team_id: options.team_id.clone(),
        user_id: options.user_id.clone(),
        budget_id: options.budget_id.clone(),
        config: parse_config(options.config.as_deref())?,
    };
    serde_json::to_vec(&body).map_err(KeysError::Json)
}

fn parse_spend(spend: Option<Decimal>) -> Result<Option<Number>, KeysError> {
    let Some(spend) = spend else {
        return Ok(None);
    };
    let number = spend
        .to_string()
        .parse::<Number>()
        .map_err(|_source| KeysError::Usage {
            message: String::from("the --spend option is not a JSON number"),
        })?;
    Ok(Some(number))
}

fn parse_aliases(text: Option<&str>) -> Result<Option<JsonValue>, KeysError> {
    let Some(text) = text else {
        return Ok(None);
    };
    let value: JsonValue = serde_json::from_str(text).map_err(|source| KeysError::Usage {
        message: format!("Invalid JSON: {source}"),
    })?;
    let Some(entries) = value.as_object() else {
        return Err(KeysError::Usage {
            message: String::from("the --aliases option must hold a JSON object"),
        });
    };
    if entries.values().any(|entry| !entry.is_string()) {
        return Err(KeysError::Usage {
            message: String::from("the --aliases option must hold a JSON object of strings"),
        });
    }
    Ok(Some(value))
}

fn parse_config(text: Option<&str>) -> Result<Option<JsonValue>, KeysError> {
    let Some(text) = text else {
        return Ok(None);
    };
    let value: JsonValue = serde_json::from_str(text).map_err(|source| KeysError::Usage {
        message: format!("Invalid JSON: {source}"),
    })?;
    if !value.is_object() {
        return Err(KeysError::Usage {
            message: String::from("the --config option must hold a JSON object"),
        });
    }
    Ok(Some(value))
}
