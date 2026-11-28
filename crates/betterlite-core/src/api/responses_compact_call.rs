pub mod call;
pub mod deps;
mod encode;
pub mod error;
mod execute;
mod logging;
mod tests;

use betterlite_models::decisions::Json;
use betterlite_openai::response_events::ResponsesReply;

use crate::api::responses_compact_http::service::{ClientSource, CompactTransport, LogContext};

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
    S: ClientSource<Client: CompactTransport>,
    F: CompactConfigFactory,
{
    execute::run(
        deps,
        call,
        call.custom_llm_provider.as_deref(),
        execute::handler_mode(call.acompact_responses.as_ref()),
        log,
    )
    .await
}

// LiteLLM's async wrapper: it detects the provider without a base, forces the marker and rewrites the id again
pub async fn acompact_responses<S, F>(
    deps: &CompactResponsesDeps<'_, S, F>,
    call: &CompactResponsesCall,
    log: &mut LogContext<'_, '_>,
) -> Result<ResponsesReply, CompactResponsesError>
where
    S: ClientSource<Client: CompactTransport>,
    F: CompactConfigFactory,
{
    let provider = match &call.custom_llm_provider {
        Some(provider) => provider.clone(),
        None => execute::preliminary_provider(deps.resolver, call)?,
    };
    let mut forced = call.clone();
    forced.custom_llm_provider = Some(provider.clone());
    forced.acompact_responses = Some(Json::Bool(true));
    let mut reply = compact_responses(deps, &forced, log).await?;
    encode::encode_response(&mut reply, &forced, &provider)?;
    Ok(reply)
}
