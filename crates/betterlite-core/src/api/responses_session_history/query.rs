use std::time::Duration;

use betterlite_db::auxiliary_repository::repository::TableError;

use crate::api::bfl::poll::PollSleeper;
use crate::api::response_id_codecs::response::decode_previous_response_id;
use crate::api::responses_session_history::error::SessionHistoryError;
use crate::api::responses_session_history::replay::replay_turn;
use crate::api::responses_session_history::{
    ChatSession, ColdStorageReader, SessionHistory, SessionLog, SpendLogQuery,
};

// the request id names one turn; every row of that turn's session comes back, oldest first
pub const SESSION_SQL: &str = "WITH matching_session AS ( SELECT session_id FROM \"LiteLLM_SpendLogs\" WHERE request_id = $1 ) SELECT * FROM \"LiteLLM_SpendLogs\" WHERE session_id IN (SELECT session_id FROM matching_session) ORDER BY \"endTime\" ASC;";

// the worker that served the turn may still be writing its row, so an empty answer is retried
pub async fn session_logs<Query, Cold, Sleep>(
    service: &SessionHistory<'_, Query, Cold, Sleep>,
    previous_response_id: &str,
) -> Result<Vec<SessionLog>, TableError>
where
    Query: SpendLogQuery + Sync,
    Cold: Sync,
    Sleep: PollSleeper + Sync,
{
    tracing::debug!("decoding response id={previous_response_id}");
    let response_id = decode_previous_response_id(previous_response_id);
    let Some(repository) = service.repository else {
        return Ok(Vec::new());
    };
    let attempts = if service.disable_spend_logs {
        1_u64
    } else {
        service.lookup.max_attempts
    };
    // asyncio.sleep returns at once for a negative delay; an endless one would hang the request
    let interval = Duration::try_from_secs_f64(service.lookup.retry_interval_seconds)
        .unwrap_or(Duration::ZERO);
    for attempt in 0..attempts {
        if attempt > 0_u64 {
            service.sleeper.sleep(interval).await;
        }
        let rows = repository.session_logs(SESSION_SQL, &response_id).await?;
        if !rows.is_empty() {
            tracing::debug!(
                "Found the following spend logs for previous response id {response_id}: {}",
                serde_json::to_string_pretty(&rows).unwrap_or_default()
            );
            return Ok(rows);
        }
    }
    tracing::debug!("Found no spend logs for previous response id {response_id}");
    Ok(Vec::new())
}

pub async fn session_messages<Query, Cold, Sleep>(
    service: &SessionHistory<'_, Query, Cold, Sleep>,
    previous_response_id: &str,
) -> Result<ChatSession, SessionHistoryError<Cold::Error>>
where
    Query: SpendLogQuery + Sync,
    Cold: ColdStorageReader + Sync,
    Sleep: PollSleeper + Sync,
{
    tracing::debug!("inside get_chat_completion_message_history_for_previous_response_id");
    let logs = session_logs(service, previous_response_id)
        .await
        .map_err(SessionHistoryError::Query)?;
    tracing::debug!("found {} spend logs for this response id", logs.len());
    let mut messages = Vec::new();
    for log in &logs {
        replay_turn(service, log, &mut messages).await?;
    }
    tracing::debug!(
        "chat_completion_message_history {}",
        serde_json::to_string_pretty(&messages).unwrap_or_default()
    );
    Ok(ChatSession {
        messages,
        litellm_session_id: logs.first().and_then(|log| log.session_id.clone()),
    })
}
