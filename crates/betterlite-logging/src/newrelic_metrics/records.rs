use std::num::ParseFloatError;
use std::ops::Mul as _;

use rust_decimal::Decimal;
use serde::Serialize;

use crate::fields::payload::StandardLoggingPayload;
use crate::fields::status::LoggingStatus;

#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error("response_time {seconds} cannot be read as a float")]
    ResponseTime {
        seconds: Decimal,
        #[source]
        source: ParseFloatError,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NewRelicMetricRecord {
    pub team_id: String,
    pub team_alias: String,
    pub model_group: String,
    pub model: String,
    pub custom_llm_provider: String,
    pub status: String,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub response_cost: Decimal,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub duration_ms: f64,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub team_max_budget: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub team_spend: Option<Decimal>,
}

pub fn metric_record_from_payload(
    payload: &StandardLoggingPayload,
) -> Result<NewRelicMetricRecord, RecordError> {
    let metadata = &payload.metadata;
    // as_f64 rounds by hand and can miss the nearest float, while parsing the decimal text rounds like Python
    let seconds = payload
        .response_time
        .to_string()
        .parse::<f64>()
        .map_err(|source| RecordError::ResponseTime {
            seconds: payload.response_time,
            source,
        })?;
    Ok(NewRelicMetricRecord {
        team_id: metadata
            .user_api_key_team_id
            .clone()
            .or_else(|| metadata.team_id.clone())
            .unwrap_or_default(),
        team_alias: metadata
            .user_api_key_team_alias
            .clone()
            .or_else(|| metadata.team_alias.clone())
            .unwrap_or_default(),
        model_group: payload.model_group.clone().unwrap_or_default(),
        model: payload.model.clone(),
        custom_llm_provider: payload.custom_llm_provider.clone().unwrap_or_default(),
        status: match payload.status {
            LoggingStatus::Success => "success".to_owned(),
            LoggingStatus::Failure => "failure".to_owned(),
        },
        response_cost: payload.response_cost,
        prompt_tokens: payload.prompt_tokens,
        completion_tokens: payload.completion_tokens,
        total_tokens: payload.total_tokens,
        // LiteLLM's duration is a float in milliseconds while the payload keeps response_time in seconds
        duration_ms: seconds.mul(1000.0_f64),
        team_max_budget: metadata.user_api_key_team_max_budget.map(|value| value.0),
        team_spend: metadata.user_api_key_team_spend.map(|value| value.0),
    })
}
