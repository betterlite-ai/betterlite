use serde_json::{Map, Value};

use crate::api::bfl::poll::PollSleeper;
use crate::api::responses_session_history::error::SessionHistoryError;
use crate::api::responses_session_history::merge::{CompletionRequest, merge_session};
use crate::api::responses_session_history::{ColdStorageReader, SessionHistory, SpendLogQuery};

// The previous turn's messages go before this request's, and its session id becomes the completion's trace id
pub async fn merged_fields<Query, Cold, Sleep>(
    history: &SessionHistory<'_, Query, Cold, Sleep>,
    previous_response_id: &str,
    mapped: &Map<String, Value>,
) -> Result<Map<String, Value>, SessionHistoryError<Cold::Error>>
where
    Query: SpendLogQuery + Sync,
    Cold: ColdStorageReader + Sync,
    Sleep: PollSleeper + Sync,
{
    let request: CompletionRequest = serde_json::from_value(Value::Object(mapped.clone()))
        .map_err(SessionHistoryError::Record)?;
    let merged = merge_session(history, previous_response_id, &request).await?;
    let mut fields = mapped.clone();
    let messages = serde_json::to_value(&merged.messages).map_err(SessionHistoryError::Record)?;
    fields.insert("messages".to_owned(), messages);
    fields.insert(
        "litellm_trace_id".to_owned(),
        merged.litellm_trace_id.map_or(Value::Null, Value::String),
    );
    Ok(fields)
}
