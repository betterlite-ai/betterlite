use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use http::Method;
use rust_decimal::Decimal;
use serde::de::DeserializeOwned;
use serde_json::Number;

use super::api::ApiError;
use super::client::KeysClient;
use super::output;
use super::wire::KeyGenerateBody;
use super::{GenerateKey, KeysError, KeysPorts, UsageError, split_commas};

const GENERATE_PATH: &str = "/key/generate";

pub(super) fn generate_key(options: &GenerateKey, ports: KeysPorts<'_>) -> Result<(), KeysError> {
    let KeysPorts {
        context,
        client,
        redactor,
        stdout,
        stderr,
    } = ports;
    let encoded = generate_body(options)?;
    let api = KeysClient {
        transport: client,
        base_url: context.base_url.trim_end_matches('/'),
        api_key: context.api_key.as_ref(),
        redactor,
    };
    match api.send(Method::POST, GENERATE_PATH, &[], Some(encoded)) {
        Ok(reply) => output::write_json(&reply.value, output::JsonText::Pretty, stdout)
            .map_err(KeysError::Json),
        Err(ApiError::Http { status, body, .. }) => {
            output::print_http_error(status, &body, stdout, stderr).map_err(KeysError::Json)?;
            Err(KeysError::Aborted)
        }
        Err(error) => Err(KeysError::Client(error)),
    }
}

fn generate_body(options: &GenerateKey) -> Result<Vec<u8>, KeysError> {
    let aliases: Option<BTreeMap<String, String>> =
        parse_object(options.aliases.as_deref(), "aliases")?;
    let body = KeyGenerateBody {
        models: split_commas(options.models.as_deref())
            .map(|parts| parts.into_iter().map(JsonValue::String).collect()),
        aliases: aliases.map(|entries| {
            entries
                .into_iter()
                .map(|(alias, model)| (alias, JsonValue::String(model)))
                .collect()
        }),
        spend: parse_spend(options.spend)?,
        duration: options.duration.clone(),
        key_alias: options.key_alias.clone(),
        team_id: options.team_id.clone(),
        user_id: options.user_id.clone(),
        budget_id: options.budget_id.clone(),
        config: parse_object(options.config.as_deref(), "config")?,
    };
    serde_json::to_vec(&body).map_err(KeysError::Json)
}

fn parse_spend(spend: Option<Decimal>) -> Result<Option<Number>, UsageError> {
    let Some(amount) = spend else {
        return Ok(None);
    };
    let number = amount
        .to_string()
        .parse::<Number>()
        .map_err(|source| UsageError::Spend { source })?;
    Ok(Some(number))
}

fn parse_object<T>(text: Option<&str>, option: &'static str) -> Result<Option<T>, UsageError>
where
    T: DeserializeOwned,
{
    let Some(raw) = text else {
        return Ok(None);
    };
    let value: JsonValue =
        serde_json::from_str(raw).map_err(|source| UsageError::InvalidJson { source })?;
    let parsed = serde_json::from_value(value)
        .map_err(|source| UsageError::OptionType { option, source })?;
    Ok(Some(parsed))
}
