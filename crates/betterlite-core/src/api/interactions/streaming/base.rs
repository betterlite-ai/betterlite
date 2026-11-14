use betterlite_interactions::types::response::InteractionsStreamChunk;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_streaming::stream_chunk_shaping::extras::strip_sse_data_from_chunk;
use chrono::{DateTime, FixedOffset};

use super::logging::InteractionsStreamLogging;
use super::records::InteractionsStreamHiddenParams;
use crate::api::interactions::contract::config::BaseInteractionsAPIConfig;

pub const SSE_DONE: &str = "[DONE]";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkOutcome {
    pub event: Option<InteractionsStreamChunk>,
    pub completed: bool,
}

impl ChunkOutcome {
    const NONE: Self = Self {
        event: None,
        completed: false,
    };
}

pub struct InteractionsStreamBase<'base, Config, Logging> {
    pub model: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub config: &'base Config,
    pub logging: &'base mut Logging,
    pub hidden_params: InteractionsStreamHiddenParams,
    pub start_time: DateTime<FixedOffset>,
    pub completed_response: Option<InteractionsStreamChunk>,
    pub finished: bool,
}

impl<Config, Logging> InteractionsStreamBase<'_, Config, Logging>
where
    Config: BaseInteractionsAPIConfig,
    Logging: InteractionsStreamLogging,
{
    pub fn process_chunk(&mut self, chunk: &str) -> Result<ChunkOutcome, Config::Error> {
        if chunk.is_empty() {
            return Ok(ChunkOutcome::NONE);
        }
        let Some(stripped) = strip_sse_data_from_chunk(Some(chunk)) else {
            return Ok(ChunkOutcome::NONE);
        };
        if stripped == SSE_DONE {
            self.finished = true;
            return Ok(ChunkOutcome::NONE);
        }
        let Ok(parsed) = serde_json::from_str::<JsonValue>(stripped) else {
            tracing::debug!(
                chunk_len = stripped.len(),
                "the interaction stream chunk is not JSON"
            );
            return Ok(ChunkOutcome::NONE);
        };
        let JsonValue::Object(fields) = parsed else {
            return Ok(ChunkOutcome::NONE);
        };
        let response = self.config.transform_streaming_response(
            self.model.as_deref(),
            &JsonValue::Object(fields),
            self.logging.logging(),
        )?;
        let completed = response.status.as_deref() == Some("completed")
            || response.event_type.as_deref() == Some("interaction.completed");
        if completed {
            self.completed_response = Some(response.clone());
        }
        Ok(ChunkOutcome {
            event: Some(response),
            completed,
        })
    }
}
