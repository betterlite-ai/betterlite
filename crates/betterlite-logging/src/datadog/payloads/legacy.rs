use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset, SecondsFormat};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Number;
use serde_with::{As, DefaultOnNull};

use crate::datadog::apm::Tracer;
use crate::datadog::tagging::{Environment, TeamMetadata, tags};
use crate::fields::timestamps::with_fraction;
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};

use super::assembly::assemble_payload;
use super::error::DatadogPayloadError;
use super::status::DataDogStatus;
use super::types::DatadogPayload;

const CLEANED_METADATA_KEYS: [&str; 3] = ["endpoint", "caching_groups", "previous_models"];

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct CallType(pub String);

impl Default for CallType {
    fn default() -> Self {
        Self("litellm.completion".to_owned())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct LegacyLoggingKwargs {
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub model: String,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub user: String,
    #[serde(default = "default_messages")]
    pub messages: Json,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub litellm_params: LegacyLitellmParams,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub optional_params: IndexMap<String, Json>,
    #[serde(
        default = "CallType::default",
        deserialize_with = "As::<DefaultOnNull>::deserialize"
    )]
    pub call_type: CallType,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub cache_hit: bool,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub response_cost: Decimal,
}

const fn default_messages() -> Json {
    Json::Null
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct LegacyLitellmParams {
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub metadata: IndexMap<String, Json>,
}

#[derive(Clone, Copy, Debug)]
pub struct LegacyLoggingEvent<'event> {
    pub kwargs: &'event LegacyLoggingKwargs,
    pub response_obj: &'event Json,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub status: DataDogStatus,
    pub generated_id: &'event str,
}

pub fn legacy_v1_logging_to_datadog(
    event: &LegacyLoggingEvent<'_>,
    environment: &Environment,
    tracer: &dyn Tracer,
) -> Result<DatadogPayload, DatadogPayloadError> {
    Ok(assemble_payload(
        environment,
        event.status,
        &base_tags(environment),
        legacy_message(event)?,
        tracer,
    ))
}

fn base_tags(environment: &Environment) -> Vec<String> {
    let team = TeamMetadata {
        user_api_key_team_alias: None,
        team_alias: None,
        user_api_key_team_id: None,
        team_id: None,
    };
    tags(environment, &[], &team)
}

fn legacy_message(event: &LegacyLoggingEvent<'_>) -> Result<String, DatadogPayloadError> {
    let response = match event.response_obj {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            return Err(DatadogPayloadError::ResponseNotObject);
        }
    };
    let usage = match response.get("usage") {
        Some(Json::Object(fields)) => Json::Object(fields.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(DatadogPayloadError::UsageNotObject);
        }
        None => return Err(DatadogPayloadError::MissingUsage),
    };
    let id = match response.get("id") {
        Some(Json::Text(id)) => id.clone(),
        Some(Json::Null) | None => event.generated_id.to_owned(),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(DatadogPayloadError::ResponseIdNotText);
        }
    };
    let record = legacy_record(event, response, usage, id);
    Ok(safe_dumps(&record, DEFAULT_MAX_RECURSE_DEPTH, None)?)
}

fn legacy_record(
    event: &LegacyLoggingEvent<'_>,
    response: &IndexMap<String, Json>,
    usage: Json,
    id: String,
) -> Json {
    let kwargs = event.kwargs;
    let format = SecondsFormat::Micros;
    let start_time = event.start_time.to_rfc3339_opts(format, false);
    let end_time = event.end_time.to_rfc3339_opts(format, false);
    let parameters = Json::Object(kwargs.optional_params.clone());
    let metadata = Json::Object(clean_metadata(&kwargs.litellm_params.metadata));
    let entries = [
        ("id", Json::Text(id)),
        ("call_type", Json::Text(kwargs.call_type.0.clone())),
        ("cache_hit", Json::Bool(kwargs.cache_hit)),
        ("start_time", Json::Text(start_time)),
        ("end_time", Json::Text(end_time)),
        ("response_time", response_time(event)),
        ("model", Json::Text(kwargs.model.clone())),
        ("user", Json::Text(kwargs.user.clone())),
        ("model_parameters", parameters),
        ("spend", number_json(kwargs.response_cost)),
        ("messages", kwargs.messages.clone()),
        ("response", Json::Object(response.clone())),
        ("usage", usage),
        ("metadata", metadata),
    ];
    Json::Object(
        entries
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    )
}

fn response_time(event: &LegacyLoggingEvent<'_>) -> Json {
    let elapsed = event.end_time.signed_duration_since(event.start_time);
    let Some(micros) = elapsed.num_microseconds() else {
        return Json::Null;
    };
    number_json(with_fraction(Decimal::new(micros, 3)))
}

fn clean_metadata(metadata: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    metadata
        .iter()
        .filter(|(key, _value)| !CLEANED_METADATA_KEYS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn number_json(value: Decimal) -> Json {
    match Number::from_str(&value.to_string()) {
        Ok(number) => Json::Number(number),
        Err(_error) => Json::Null,
    }
}
