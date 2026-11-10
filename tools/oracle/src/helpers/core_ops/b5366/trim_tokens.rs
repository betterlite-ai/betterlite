use betterlite_core::message_utilities::trimming::shorten::shorten_message_to_fit_limit;
use betterlite_core::message_utilities::trimming::steps::can_add_message;
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
    CanAddMessage(CanAddArgs),
    GetTokenCount(TokenCountArgs),
    ShortenMessageToFitLimit(ShortenArgs),
    TrimMessages(TrimArgs),
}

#[derive(Debug, Deserialize)]
pub(crate) struct CanAddArgs {
    message: Json,
    messages: Vec<Json>,
    max_tokens: i64,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TokenCountArgs {
    messages: Vec<Json>,
    #[serde(default)]
    model: Option<String>,
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
        Call::CanAddMessage(args) => can_add(args, &trimmer).await,
        Call::GetTokenCount(args) => token_count(args, &trimmer).await,
        Call::ShortenMessageToFitLimit(args) => shorten(args, &trimmer).await,
        Call::TrimMessages(args) => trim(args, &trimmer).await,
    }
}

async fn can_add(args: &CanAddArgs, trimmer: &Trimmer<'_>) -> HelperResult {
    let model = args.model.as_deref().unwrap_or_default();
    can_add_message(
        trimmer,
        &args.message,
        &args.messages,
        args.max_tokens,
        model,
    )
    .await
    .map(|fits| json!(fits))
    .map_err(|error| errors::value_error(error.to_string()))
}

async fn token_count(args: &TokenCountArgs, trimmer: &Trimmer<'_>) -> HelperResult {
    trimmer
        .get_token_count(&args.messages, args.model.as_deref().unwrap_or_default())
        .await
        .map(|count| json!(count))
        .map_err(|error| errors::value_error(error.to_string()))
}

async fn shorten(args: &ShortenArgs, trimmer: &Trimmer<'_>) -> HelperResult {
    let model = args.model.as_deref().unwrap_or_default();
    shorten_message_to_fit_limit(
        trimmer,
        &args.message,
        args.tokens_needed,
        model,
        args.raise_error_on_max_limit,
    )
    .await
    .map(|shortened| json!(shortened))
    .map_err(|error| errors::value_error(error.to_string()))
}

async fn trim(args: &TrimArgs, trimmer: &Trimmer<'_>) -> HelperResult {
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
