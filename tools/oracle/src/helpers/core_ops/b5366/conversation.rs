use betterlite_core::message_utilities::trimming::steps::{
    TokenBudget, attempt_message_addition, process_messages, process_system_message,
};
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
    ProcessSystemMessage(SystemMessageArgs),
    ProcessMessages(MessagesArgs),
    AttemptMessageAddition(AttemptArgs),
}

#[derive(Debug, Deserialize)]
pub(crate) struct SystemMessageArgs {
    system_message: String,
    max_tokens: i64,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MessagesArgs {
    messages: Vec<Json>,
    max_tokens: i64,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AttemptArgs {
    final_messages: Vec<Json>,
    message: Json,
    available_tokens: i64,
    max_tokens: i64,
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
        Call::ProcessSystemMessage(args) => system_message(args, &trimmer).await,
        Call::ProcessMessages(args) => messages(args, &trimmer).await,
        Call::AttemptMessageAddition(args) => attempt(args, &trimmer).await,
    }
}

async fn system_message(args: &SystemMessageArgs, trimmer: &Trimmer<'_>) -> HelperResult {
    let model = args.model.as_deref().unwrap_or_default();
    process_system_message(trimmer, &args.system_message, args.max_tokens, model)
        .await
        .map(|(event, remaining)| json!([event, remaining]))
        .map_err(|error| errors::value_error(error.to_string()))
}

async fn messages(args: &MessagesArgs, trimmer: &Trimmer<'_>) -> HelperResult {
    let model = args.model.as_deref().unwrap_or_default();
    process_messages(trimmer, &args.messages, args.max_tokens, model)
        .await
        .map(|processed| json!(processed))
        .map_err(|error| errors::value_error(error.to_string()))
}

async fn attempt(args: &AttemptArgs, trimmer: &Trimmer<'_>) -> HelperResult {
    let budget = TokenBudget {
        available: args.available_tokens,
        max: args.max_tokens,
    };
    let model = args.model.as_deref().unwrap_or_default();
    attempt_message_addition(trimmer, &args.final_messages, &args.message, budget, model)
        .await
        .map(|added| json!(added))
        .map_err(|error| errors::value_error(error.to_string()))
}
