use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_logging::verbose_hooks::VerboseHooks;
use betterlite_redact::secret::Redactor;
use betterlite_redact::structured::Payload;
use betterlite_streaming::streamed_response_assembly::records::Chunk;
use betterlite_streaming::streamed_response_assembly::response::Response;
use betterlite_streaming::streamed_response_assembly::{Request, stream_chunk_builder};

use crate::purview_dlp_evaluation::MicrosoftPurviewDLPGuardrail;
use crate::purview_dlp_evaluation::completion_text::{
    CompletionResponse, completion_response_text_parts,
};
use crate::purview_dlp_evaluation::error_handling;
use crate::purview_dlp_evaluation::stream_assembly::assemble_responses_stream;
use crate::purview_dlp_evaluation::user_id_blocking::resolve_user_id_for_blocking;

use super::context::HookContext;
use super::error::PurviewHookError;
use super::scan::{PartsScan, scan_parts};
use super::{Data, conversion, response_parts};
use services::{NoCosts, SilentConsole, SilentCounter, SilentLogger};

pub(super) const ASSEMBLY_ERROR: &str =
    "Microsoft Purview DLP: Unable to assemble streamed response for scanning; blocking response.";
pub(super) const INCOMPLETE_ERROR: &str = "Microsoft Purview DLP: Incomplete Responses API stream — no final response event received for DLP scanning; blocking response.";

mod services;
mod tests;

async fn assemble_chat(chunks: &[GuardrailValue], redactor: &Arc<Redactor>) -> Option<Response> {
    let mut parsed = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let encoded = serde_json::to_string(chunk).ok()?;
        parsed.push(serde_json::from_str::<Chunk>(&encoded).ok()?);
    }
    let hooks = VerboseHooks {
        logger: SilentLogger,
        console: SilentConsole,
        set_verbose: Payload::Bool(false),
        redactor: Arc::clone(redactor),
    };
    let request = Request {
        chunks: Some(&parsed),
        messages: None,
        count_prompt_tokens: None,
        logging: None,
        costs: &NoCosts,
        counter: &SilentCounter,
        metadata: None,
        now: 0,
    };
    match stream_chunk_builder(&request, &hooks).await {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!("Purview DLP: streamed response assembly failed: {error}");
            None
        }
    }
}

async fn response_text_parts(
    chunks: &[GuardrailValue],
    redactor: &Arc<Redactor>,
) -> Result<Vec<String>, PurviewHookError> {
    let events: Vec<_> = chunks
        .iter()
        .filter_map(|chunk| serde_json::from_str(&serde_json::to_string(chunk).ok()?).ok())
        .collect();
    let (marked, assembled) = assemble_responses_stream(&events);
    if marked {
        let Some(response) = assembled else {
            return Err(PurviewHookError::from(error_handling::message_error(
                INCOMPLETE_ERROR,
            )));
        };
        return Ok(completion_response_text_parts(
            &CompletionResponse::Responses(&response),
        ));
    }
    let Some(response) = assemble_chat(chunks, redactor).await else {
        return Err(PurviewHookError::from(error_handling::message_error(
            ASSEMBLY_ERROR,
        )));
    };
    let Ok(value) =
        serde_json::to_string(&response).and_then(|encoded| serde_json::from_str(&encoded))
    else {
        return Err(PurviewHookError::from(error_handling::message_error(
            ASSEMBLY_ERROR,
        )));
    };
    Ok(response_parts::parts(&value)?)
}

// ast-grep-ignore: no-long-names
pub async fn async_post_call_streaming_iterator_hook(
    guardrail: &MicrosoftPurviewDLPGuardrail,
    chunks: Vec<GuardrailValue>,
    data: &mut Data,
    context: &mut HookContext<'_>,
) -> Result<Vec<GuardrailValue>, PurviewHookError> {
    // Nothing leaves the buffered stream until the scan passes: the caller gets the chunks back only on Ok.
    let request_data = conversion::object(data);
    let user_id = resolve_user_id_for_blocking(&guardrail.base, &request_data, &context.identity)?;
    if chunks.is_empty() {
        return Ok(chunks);
    }
    let parts = response_text_parts(&chunks, &context.redactor).await?;
    if !parts.is_empty() {
        let scan = PartsScan {
            user_id: &user_id,
            parts: &parts,
            block_on_violation: true,
            request_data: &request_data,
        };
        let targets = context.record_targets();
        scan_parts(guardrail, data, &scan, targets).await?;
    }
    Ok(chunks)
}
