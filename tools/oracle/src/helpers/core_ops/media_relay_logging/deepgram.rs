use betterlite_config::cost_adjustments::{AdjustmentKind, resolve_adjustment_entries};
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use serde_json::{Map, Value, json};
use tokio::runtime::Builder;

use super::services::{Adjustments, listen_outcome};
use super::{DeepgramCall, failure, relay_answer};

// The transcription text and the billed duration, then what every relay handler leaves for the spend logger.
pub(super) fn deepgram_answer(call: &DeepgramCall) -> Result<Value, Box<HelperError>> {
    let adjustments = Adjustments {
        discount: resolve_adjustment_entries(
            AdjustmentKind::Discount,
            "litellm_settings.cost_discount_config",
            call.cost_discount_config.clone(),
        )
        .map_err(failure)?,
        margin: resolve_adjustment_entries(
            AdjustmentKind::Margin,
            "litellm_settings.cost_margin_config",
            call.cost_margin_config.clone(),
        )
        .map_err(failure)?,
    };
    let kwargs = PassThroughLoggingKwargs {
        entries: call.kwargs.clone(),
    };
    let runtime = Builder::new_current_thread().build().map_err(failure)?;
    let listen = runtime.block_on(listen_outcome(
        &call.catalog,
        &adjustments,
        &call.upstream_url,
        &call.messages,
        &kwargs,
    ))?;
    let mut answer = Map::new();
    answer.insert("text".to_owned(), json!(transcript_text(&listen.outcome)));
    answer.insert(
        "billed_seconds".to_owned(),
        json!(listen.billed_seconds.normalize().to_string()),
    );
    Ok(relay_answer(answer, &listen.outcome, &listen.spend))
}

fn transcript_text(outcome: &HandlerOutcome) -> String {
    let Some(PassThroughResponse::Transcription(Json::Object(fields))) = &outcome.result else {
        return String::new();
    };
    let Some(Json::Text(text)) = fields.get("text") else {
        return String::new();
    };
    text.clone()
}
