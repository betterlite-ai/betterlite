use betterlite_core::message_utilities::trimming::steps::can_add_message;
use betterlite_core::message_utilities::trimming::{DEFAULT_MAX_TRIM_ATTEMPTS, Trimmer};
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

pub(super) async fn run(call: &Call, session: &Session) -> HelperResult {
    let trimmer = Trimmer {
        counter: &session.counter,
        catalog: session.catalog.as_ref(),
        max_attempts: DEFAULT_MAX_TRIM_ATTEMPTS,
    };
    match call {
        Call::CanAddMessage(args) => {
            let model = args.model.as_deref().unwrap_or_default();
            let fits = can_add_message(
                &trimmer,
                &args.message,
                &args.messages,
                args.max_tokens,
                model,
            )
            .await
            .map_err(|error| errors::value_error(error.to_string()))?;
            Ok(json!(fits))
        }
        Call::GetTokenCount(args) => trimmer
            .get_token_count(&args.messages, args.model.as_deref().unwrap_or_default())
            .await
            .map(|count| json!(count))
            .map_err(|error| errors::value_error(error.to_string())),
    }
}
