use std::error::Error as StdError;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::messages_native_stream::retention::RetentionBudget;
use tokio::sync::mpsc::Receiver;

use super::error::NomaLifecycleError;
use super::hooks::{ScanSubject, scan};
use super::stream_error::NomaStreamError;
use super::{NomaGuardrail, RequestData};
use crate::cato::error::ModelStreamError;
use crate::noma_scan_policy::error::NomaError;

// What stream_chunk_builder returns: only a ModelResponse is scanned, a text completion or nothing is not
#[derive(Debug, Clone, PartialEq)]
pub enum AssembledStream {
    Model(GuardrailValue),
    Other,
}

// The two LiteLLM streaming helpers the hook leans on, injected so this guardrail owns neither
pub trait StreamAssembly {
    type Error: StdError + Send + 'static;

    // LiteLLM's stream_chunk_builder
    fn assemble(&self, chunks: &[GuardrailValue]) -> Result<AssembledStream, Self::Error>;

    // LiteLLM's MockResponseIterator: the response as the one chunk the iterator yields
    fn replay(&self, response: GuardrailValue) -> Result<GuardrailValue, Self::Error>;
}

pub struct StreamHookRequest<'request> {
    pub user_auth: &'request RequestData,
    pub data: &'request mut RequestData,
}

// The bytes a chunk carries, which is what the buffer's byte ceiling counts
fn carried_bytes(chunk: &GuardrailValue) -> usize {
    match chunk {
        GuardrailValue::Text(text) => text.len(),
        GuardrailValue::Array(items) => items
            .iter()
            .map(carried_bytes)
            .fold(0_usize, usize::saturating_add),
        GuardrailValue::Object(fields) => fields
            .iter()
            .map(|(name, item)| name.len().saturating_add(carried_bytes(item)))
            .fold(0_usize, usize::saturating_add),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_) => 8_usize,
    }
}

// The buffer holds untrusted provider output, so the API boundary ceilings bound it
async fn buffer_stream<A>(
    mut response: Receiver<Result<GuardrailValue, ModelStreamError>>,
) -> Result<Vec<GuardrailValue>, NomaStreamError<A::Error>>
where
    A: StreamAssembly,
{
    let limits = ApiLimits::default();
    let mut budget = RetentionBudget {
        max_bytes: limits.max_json_bytes,
        max_chunks: limits.max_items,
        bytes: 0,
        chunks: 0,
    };
    let mut chunks = Vec::new();
    while let Some(received) = response.recv().await {
        let chunk = received.map_err(|source| NomaStreamError::<A::Error>::Model { source })?;
        budget.accept(carried_bytes(&chunk)).map_err(|source| {
            NomaStreamError::<A::Error>::OverBudget {
                bytes: limits.max_json_bytes,
                chunks: limits.max_items,
                source,
            }
        })?;
        chunks.push(chunk);
    }
    Ok(chunks)
}

// ast-grep-ignore: no-long-names
pub async fn async_post_call_streaming_iterator_hook<A>(
    guardrail: &NomaGuardrail,
    request: StreamHookRequest<'_>,
    response: Receiver<Result<GuardrailValue, ModelStreamError>>,
    assembly: &A,
) -> Result<Vec<GuardrailValue>, NomaStreamError<A::Error>>
where
    A: StreamAssembly + Sync,
{
    // LiteLLM yields nothing until the whole stream is buffered, so the chunks to yield come back as one list
    let chunks = buffer_stream::<A>(response).await?;
    if chunks.is_empty() {
        return Ok(chunks);
    }
    let assembled = assembly
        .assemble(&chunks)
        .map_err(|source| NomaStreamError::<A::Error>::Assembly { source })?;
    let AssembledStream::Model(mut model_response) = assembled else {
        return Ok(chunks);
    };
    let subject = ScanSubject::LlmResponse(&mut model_response);
    let event_type = Some(GuardrailEventHooks::PostCall);
    if let Err(error) = scan(
        guardrail,
        request.data,
        request.user_auth,
        subject,
        event_type,
    )
    .await
    {
        if guardrail.block_failures
            || matches!(error, NomaLifecycleError::Noma(NomaError::Blocked(_)))
        {
            return Err(NomaStreamError::<A::Error>::Lifecycle(error));
        }
        tracing::error!("Noma streaming post-call hook failed: {error}");
        return Ok(chunks);
    }
    let replayed = assembly
        .replay(model_response)
        .map_err(|source| NomaStreamError::<A::Error>::Assembly { source })?;
    Ok(vec![replayed])
}
