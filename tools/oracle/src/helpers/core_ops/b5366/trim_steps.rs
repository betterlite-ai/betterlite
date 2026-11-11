use betterlite_core::message_utilities::trimming::shorten::shorten_message_to_fit_limit;
use betterlite_core::message_utilities::trimming::{
    DEFAULT_MAX_TRIM_ATTEMPTS, DEFAULT_TRIM_RATIO, TrimRequest, Trimmer,
};
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::json;

use super::errors;
use super::fixture::Session;
use crate::helpers::HelperResult;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Call {
    ShortenMessageToFitLimit(ShortenArgs),
    TrimMessages(TrimArgs),
}

#[derive(Debug, Deserialize)]
pub(crate) struct ShortenArgs {
    message: Json,
    tokens_needed: i64,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    raise_error_on_max_limit: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TrimArgs {
    messages: Vec<Json>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    trim_ratio: Option<f64>,
    #[serde(default)]
    return_response_tokens: bool,
    #[serde(default)]
    max_tokens: Option<i64>,
}

pub(super) async fn run(call: &Call, session: &Session) -> HelperResult {
    let trimmer = Trimmer {
        counter: &session.counter,
        catalog: session.catalog.as_ref(),
        max_attempts: DEFAULT_MAX_TRIM_ATTEMPTS,
    };
    match call {
        Call::ShortenMessageToFitLimit(args) => {
            let shortened = shorten_message_to_fit_limit(
                &trimmer,
                &args.message,
                args.tokens_needed,
                args.model.as_deref().unwrap_or_default(),
                args.raise_error_on_max_limit,
            )
            .await
            .map_err(|error| errors::value_error(error.to_string()))?;
            Ok(json!(shortened))
        }
        Call::TrimMessages(args) => {
            let request = TrimRequest {
                messages: &args.messages,
                model: args.model.as_deref().unwrap_or_default(),
                trim_ratio: args.trim_ratio.unwrap_or(DEFAULT_TRIM_RATIO),
                return_response_tokens: args.return_response_tokens,
                max_tokens: args.max_tokens,
            };
            let outcome = trimmer.trim_messages(&request).await;
            match outcome.response_tokens {
                Some(tokens) => Ok(json!([outcome.messages, tokens])),
                None => Ok(json!(outcome.messages)),
            }
        }
    }
}
