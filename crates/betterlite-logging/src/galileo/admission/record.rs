use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset, LocalResult, Utc};
use indexmap::IndexMap;
use rust_decimal::{Decimal, RoundingStrategy};

use super::{AdmissionError, CallbackKwargs, StandardLoggingObject, SuccessEvent};
use crate::galileo_payloads::LlmResponse;
use crate::galileo_payloads::content::{self, Level};
use crate::galileo_payloads::timestamps::{RawTimestamp, format_created_at};

struct Window {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

struct TokenCounts {
    input: i64,
    output: i64,
    total: i64,
}

struct RecordLabels {
    node_type: String,
    model: String,
}

// str(slo.get("call_type") or kwargs.get("call_type") or "litellm"); the model falls back the same way to "-".
fn record_labels(slo: &StandardLoggingObject, kwargs: &CallbackKwargs) -> RecordLabels {
    let call_type = slo
        .call_type
        .as_deref()
        .or(kwargs.content.call_type.as_deref())
        .unwrap_or("litellm");
    let model = slo
        .model
        .as_deref()
        .or(kwargs.model.as_deref())
        .unwrap_or("-");
    RecordLabels {
        node_type: call_type.to_owned(),
        model: model.to_owned(),
    }
}

// litellm.integrations.galileo.GalileoObserve._async_log_success_event_impl's LLMResponse construction; None when the fallback pair is missing so the caller drops the callback.
pub(super) fn success_record(
    event: &SuccessEvent<'_>,
    slo: &StandardLoggingObject,
) -> Result<Option<IndexMap<String, Json>>, AdmissionError> {
    let kwargs = event.kwargs;
    let (input_text, output_text, messages) = content::galileo_input_output_content(
        &kwargs.content,
        event.response,
        Level::Default,
        None,
    );
    let Some(window) = event_times(slo, event)? else {
        return Ok(None);
    };
    let tokens = token_counts(slo);
    let labels = record_labels(slo, kwargs);
    let record = LlmResponse {
        latency_ms: latency_ms(&window),
        status_code: 200,
        input_text,
        output_text,
        node_type: labels.node_type,
        model: labels.model,
        num_input_tokens: tokens.input,
        num_output_tokens: tokens.output,
        num_total_tokens: tokens.total,
        cost: slo.response_cost,
        output_logprobs: None,
        tags: None,
        user_metadata: None,
        created_at: format_created_at(
            RawTimestamp::Aware(window.start.fixed_offset()),
            event.local_offset,
        ),
    };
    Ok(Some(record.to_record(record_messages(&messages))))
}

// slo.get(key) or 0 per field, and a zero total is summed from the parts when either part is set.
fn token_counts(slo: &StandardLoggingObject) -> TokenCounts {
    let input = slo.prompt_tokens.unwrap_or(0);
    let output = slo.completion_tokens.unwrap_or(0);
    let mut total = slo.total_tokens.unwrap_or(0);
    if total == 0 && (input != 0 || output != 0) {
        total = input.saturating_add(output);
    }
    TokenCounts {
        input,
        output,
        total,
    }
}

// max(0, int((end - start).total_seconds() * 1000)) with the microsecond resolution both sides carry.
fn latency_ms(window: &Window) -> i64 {
    window
        .end
        .signed_duration_since(window.start)
        .num_microseconds()
        .and_then(|micros| micros.max(0).checked_div(1_000))
        .unwrap_or(0)
}

// The standard_logging_object times win as Unix seconds; a missing pair falls back to the callback's datetimes.
fn event_times(
    slo: &StandardLoggingObject,
    event: &SuccessEvent<'_>,
) -> Result<Option<Window>, AdmissionError> {
    if let (Some(start), Some(end)) = (slo.start_time, slo.end_time) {
        let start_at = utc_from_epoch_seconds(start)?;
        let end_at = utc_from_epoch_seconds(end)?;
        return Ok(Some(Window {
            start: start_at,
            end: end_at,
        }));
    }
    tracing::debug!(
        "Galileo Logger: standard_logging_object missing startTime/endTime, falling back to start_time/end_time params"
    );
    let (Some(start), Some(end)) = (event.start_time, event.end_time) else {
        return Ok(None);
    };
    Ok(Some(Window {
        start: utc_time(start, event.local_offset),
        end: utc_time(end, event.local_offset),
    }))
}

// datetime.fromtimestamp(seconds, tz=utc) rounds to the microsecond the way CPython's fromtimestamp does.
fn utc_from_epoch_seconds(seconds: Decimal) -> Result<DateTime<Utc>, AdmissionError> {
    let micros = seconds
        .round_dp_with_strategy(6, RoundingStrategy::MidpointNearestEven)
        .checked_mul(Decimal::from(1_000_000_u64))
        .and_then(|value| i64::try_from(value).ok())
        .ok_or(AdmissionError::Timestamp)?;
    DateTime::<Utc>::from_timestamp_micros(micros).ok_or(AdmissionError::Timestamp)
}

// A naive callback datetime is local wall time, which the callback's injected offset turns into UTC; an aware one converts directly.
fn utc_time(timestamp: RawTimestamp, local_offset: FixedOffset) -> DateTime<Utc> {
    match timestamp {
        RawTimestamp::Naive(naive) => match naive.and_local_timezone(local_offset) {
            LocalResult::Single(local) | LocalResult::Ambiguous(local, _) => {
                local.with_timezone(&Utc)
            }
            // A fixed offset neither skips nor repeats a wall time, so no naive value lands here.
            LocalResult::None => DateTime::from_naive_utc_and_offset(naive, Utc),
        },
        RawTimestamp::Aware(aware) => aware.with_timezone(&Utc),
    }
}

// A prompt-shaped messages value unwraps to its "messages" entry; anything else stays as is, and only a non-empty list reaches the record.
fn record_messages(messages: &Json) -> Option<Json> {
    let candidate = match messages {
        Json::Object(fields) => fields.get("messages")?,
        other @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            other
        }
    };
    match candidate {
        Json::List(items) if !items.is_empty() => Some(Json::List(items.clone())),
        Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::Text(_)
        | Json::List(_)
        | Json::Object(_) => None,
    }
}
