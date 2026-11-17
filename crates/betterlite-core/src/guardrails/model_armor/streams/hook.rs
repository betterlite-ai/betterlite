use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrails::streaming_rewrite::shapes::RequestData;

use super::assembly::{Assembly, assemble};
use super::chunk::Chunk;
use super::errors::{HttpError, StreamError, frame_error};
use super::outcome::resolve_outcome;
use super::responses::content_to_scan;
use super::rewrite::apply_sanitized;
use super::surface::{Surface, classify, is_terminal_error};
use super::wire::{ScanFailure, StreamWire};

const METADATA_KEY: &str = "metadata";
const LITELLM_METADATA_KEY: &str = "litellm_metadata";
const RESPONSE_KEY: &str = "_model_armor_response";
const STATUS_KEY: &str = "_model_armor_status";
const BLOCKED: &str = "blocked";
const SUCCESS: &str = "success";
const BLOCK_MESSAGE: &str = "Streaming response blocked by Model Armor";
const ASSEMBLY_MESSAGE: &str = "streamed response could not be assembled for scanning, blocking it";

// The two optional parameters that decide how a failure and a sanitized match are handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamSettings {
    pub fail_on_error: bool,
    pub mask_response_content: bool,
}
// Everything a buffered run reads and writes besides the chunks it decides about.
pub struct StreamHook<'hook, W: StreamWire> {
    pub wire: &'hook W,
    pub request_data: &'hook mut RequestData,
    pub guardrail_name: &'hook str,
    pub settings: StreamSettings,
    pub chunks: Vec<Chunk>,
}

// LiteLLM's async_post_call_streaming_iterator_hook: buffer the whole stream, scan it, then yield it whole or refuse it.
pub async fn buffered_stream<W>(
    hook: StreamHook<'_, W>,
) -> Result<Vec<Chunk>, StreamError<W::Failure>>
where
    W: StreamWire,
{
    if hook.chunks.is_empty() || is_terminal_error(hook.wire, &hook.chunks) {
        return Ok(hook.chunks);
    }
    let surface = classify(hook.wire, &hook.chunks);
    let Some(assembled) = assemble(hook.wire, surface, &hook.chunks).await else {
        return assembly_failure(hook, surface);
    };
    let content = content_to_scan(hook.wire, surface, &assembled, &hook.chunks)
        .map_err(StreamError::Wiring)?;
    if content.is_empty() {
        return Ok(hook.chunks);
    }
    let armor = match hook.wire.scan(&content).await {
        Ok(armor) => armor,
        Err(failure) => return scan_failure(hook, surface, failure),
    };
    finish_scan(hook, surface, &assembled, &content, &armor)
}

fn assembly_failure<W>(
    hook: StreamHook<'_, W>,
    surface: Surface,
) -> Result<Vec<Chunk>, StreamError<W::Failure>>
where
    W: StreamWire,
{
    if !hook.settings.fail_on_error {
        tracing::warn!(
            "Model Armor: streamed response could not be assembled for scanning, forwarding it unscanned because fail_on_error is disabled"
        );
        return Ok(hook.chunks);
    }
    hook.wire
        .applied_guardrail(hook.request_data, hook.guardrail_name);
    let detail = format!("{}: {ASSEMBLY_MESSAGE}", hook.guardrail_name);
    let error = HttpError {
        status: 500,
        detail: GuardrailValue::Text(detail),
    };
    frame_error(hook.wire, surface, &error)
}

fn scan_failure<W>(
    hook: StreamHook<'_, W>,
    surface: Surface,
    failure: ScanFailure<W::Failure>,
) -> Result<Vec<Chunk>, StreamError<W::Failure>>
where
    W: StreamWire,
{
    match failure {
        ScanFailure::Api { detail } if hook.settings.fail_on_error => {
            let error = HttpError {
                status: 500,
                detail: GuardrailValue::Text(detail),
            };
            frame_error(hook.wire, surface, &error)
        }
        ScanFailure::Api { .. } => Ok(hook.chunks),
        ScanFailure::Unexpected(unexpected) => {
            if hook.settings.fail_on_error {
                Err(StreamError::Wiring(unexpected))
            } else {
                Ok(hook.chunks)
            }
        }
    }
}

fn finish_scan<W>(
    hook: StreamHook<'_, W>,
    surface: Surface,
    assembled: &Assembly,
    content: &str,
    armor: &GuardrailValue,
) -> Result<Vec<Chunk>, StreamError<W::Failure>>
where
    W: StreamWire,
{
    let outcome = resolve_outcome(
        hook.wire,
        hook.settings.mask_response_content,
        content,
        assembled,
        armor,
    );
    record_metadata(
        &mut *hook.request_data,
        hook.wire.logging_response(armor),
        outcome.blocked,
    );
    hook.wire
        .applied_guardrail(&mut *hook.request_data, hook.guardrail_name);
    if outcome.blocked {
        let detail = hook.wire.block_detail(BLOCK_MESSAGE, armor);
        return frame_error(
            hook.wire,
            surface,
            &HttpError {
                status: 400,
                detail,
            },
        );
    }
    let Some(sanitized) = outcome.sanitized.as_deref() else {
        return Ok(hook.chunks);
    };
    rewritten_stream(hook, surface, assembled, sanitized)
}

fn rewritten_stream<W>(
    hook: StreamHook<'_, W>,
    surface: Surface,
    assembled: &Assembly,
    sanitized: &str,
) -> Result<Vec<Chunk>, StreamError<W::Failure>>
where
    W: StreamWire,
{
    let Assembly::Model(body) = assembled else {
        return Ok(hook.chunks);
    };
    let mut rewritten = body.clone();
    apply_sanitized(&mut rewritten, sanitized);
    match hook.wire.emit_sanitized(surface, &rewritten) {
        Ok(items) => Ok(items),
        Err(error) => {
            if hook.settings.fail_on_error {
                Err(StreamError::Wiring(error))
            } else {
                Ok(hook.chunks)
            }
        }
    }
}

fn record_metadata(data: &mut RequestData, logging: GuardrailValue, blocked: bool) {
    let key = if data.contains_key(LITELLM_METADATA_KEY) {
        LITELLM_METADATA_KEY
    } else {
        METADATA_KEY
    };
    let mut bucket = match data.remove(key) {
        Some(GuardrailValue::Object(fields)) => fields,
        _ => BTreeMap::new(),
    };
    bucket.insert(RESPONSE_KEY.to_owned(), logging);
    let status = if blocked { BLOCKED } else { SUCCESS };
    bucket.insert(
        STATUS_KEY.to_owned(),
        GuardrailValue::Text(status.to_owned()),
    );
    data.insert(key.to_owned(), GuardrailValue::Object(bucket));
}
