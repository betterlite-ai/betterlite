pub mod call;
mod convert;
pub mod deps;
mod encode;
pub mod error;
mod execute;
mod logging;
mod tests;

use betterlite_models::decisions::Json;
use betterlite_openai::response_events::{ReplyBody, ResponsesReply};

use crate::api::response_id_traversal::response_id::rewrite_typed_response;
use crate::api::responses_compact_http::service::{ClientKind, ClientSource, LogContext};

use self::call::CompactResponsesCall;
use self::deps::{CompactConfigFactory, CompactResponsesDeps};
use self::error::CompactResponsesError;

// LiteLLM's sync wrapper: the caller's marker, read with `is True`, picks the async handler
pub async fn compact_responses<S, F>(
    deps: &CompactResponsesDeps<'_, S, F>,
    call: &CompactResponsesCall,
    log: &mut LogContext<'_, '_>,
) -> Result<ResponsesReply, CompactResponsesError>
where
    S: ClientSource,
    F: CompactConfigFactory,
{
    let mode = handler_mode(call.acompact_responses.as_ref());
    let completed =
        execute::run(deps, call, call.custom_llm_provider.as_deref(), mode, log).await?;
    Ok(completed.reply)
}

// LiteLLM's async wrapper: it detects the provider without a base, forces the marker and rewrites the id again
pub async fn acompact_responses<S, F>(
    deps: &CompactResponsesDeps<'_, S, F>,
    call: &CompactResponsesCall,
    log: &mut LogContext<'_, '_>,
) -> Result<ResponsesReply, CompactResponsesError>
where
    S: ClientSource,
    F: CompactConfigFactory,
{
    let detected = match &call.custom_llm_provider {
        Some(provider) => provider.clone(),
        None => execute::preliminary_provider(deps.resolver, call)?,
    };
    let mut completed =
        execute::run(deps, call, Some(&detected), ClientKind::Asynchronous, log).await?;
    if let ReplyBody::Typed(response) = &mut completed.reply.response {
        let metadata = encode::affinity_metadata(call.kwargs.get("litellm_metadata"))?;
        rewrite_typed_response(response.as_mut(), Some(&detected), &metadata)?;
    }
    Ok(completed.reply)
}

const fn handler_mode(marker: Option<&Json>) -> ClientKind {
    if matches!(marker, Some(Json::Bool(true))) {
        ClientKind::Asynchronous
    } else {
        ClientKind::Sync
    }
}
