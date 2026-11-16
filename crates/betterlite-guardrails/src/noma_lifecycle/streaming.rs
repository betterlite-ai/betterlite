use std::error::Error as StdError;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use tokio::sync::mpsc::Receiver;

use super::error::NomaLifecycleError;
use super::hooks::ScanSubject;
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

impl NomaGuardrail {
    // ast-grep-ignore: no-long-names
    pub async fn async_post_call_streaming_iterator_hook<A>(
        &self,
        request: StreamHookRequest<'_>,
        mut response: Receiver<Result<GuardrailValue, ModelStreamError>>,
        assembly: &A,
    ) -> Result<Vec<GuardrailValue>, NomaStreamError<A::Error>>
    where
        A: StreamAssembly + Sync,
    {
        // LiteLLM yields nothing until the whole stream is buffered, so the chunks to yield come back as one list
        let mut chunks = Vec::new();
        while let Some(received) = response.recv().await {
            let chunk = received.map_err(|source| NomaStreamError::<A::Error>::Model { source })?;
            chunks.push(chunk);
        }
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
        if let Err(error) = self
            .scan(request.data, request.user_auth, subject, event_type)
            .await
        {
            if self.block_failures
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
}
