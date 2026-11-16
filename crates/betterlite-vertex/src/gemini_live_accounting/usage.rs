use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::Usage;
use betterlite_prompts::arguments::json::Object;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::error::LiveAccountingError;
use crate::gemini_live_client::records::ServerMessage;
use crate::gemini_live_client::state::LiveSessionState;
use crate::gemini_usage::metadata::UsageMetadata;
use crate::gemini_usage::usage::{apply_grounding_counters, calculate_usage};

// The frame's own usageMetadata is taken out so a frame that closes two responses bills one of them, and it wins even when empty
pub fn consume_usage_metadata(
    state: &mut LiveSessionState,
    frame: &mut ServerMessage,
) -> Option<Object> {
    if let Some(in_frame) = frame.usage_metadata.take() {
        state.pending_usage_metadata = None;
        return Some(in_frame);
    }
    state.pending_usage_metadata.take()
}

pub fn chat_usage(
    reported: Option<&Object>,
    grounding_metadata: Option<&Object>,
) -> Result<Usage, LiveAccountingError> {
    let Some(reported_usage) = reported else {
        return Ok(Usage::default());
    };
    let metadata: UsageMetadata = reencode(reported_usage)?;
    let mut usage = calculate_usage(Some(&metadata), &[])?;
    if let Some(grounding) = grounding_metadata {
        let converted: Json = reencode(grounding)?;
        apply_grounding_counters(&mut usage, &[converted]);
    }
    Ok(usage)
}

pub(super) fn reencode<Source, Target>(value: &Source) -> Result<Target, LiveAccountingError>
where
    Source: Serialize,
    Target: DeserializeOwned,
{
    let bytes = serde_json::to_vec(value).map_err(LiveAccountingError::Convert)?;
    serde_json::from_slice(&bytes).map_err(LiveAccountingError::Convert)
}
