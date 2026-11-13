use betterlite_context::call_context::CallContext;

use super::error::DispatchError;
use super::log::CallOutcome;
use super::{Dispatch, Entry};
use crate::operation::embedding::{EmbeddingCallInput, EmbeddingRequest, EmbeddingResult};

pub async fn execute<Metadata, Client>(
    dispatch: &Dispatch,
    request: EmbeddingRequest,
    context: &CallContext<'_, Metadata, Client>,
) -> Result<EmbeddingResult, DispatchError>
where
    Metadata: Sync,
    Client: Sync,
{
    let terminal = dispatch.terminal(Entry::Embedding, &request.call_id, context);
    match run(dispatch, &request, context).await {
        Ok(result) => {
            terminal.record(CallOutcome::Succeeded, None, None);
            Ok(result)
        }
        Err(error) => {
            terminal.record(CallOutcome::Failed, Some(&error), None);
            Err(error)
        }
    }
}

async fn run<Metadata, Client>(
    dispatch: &Dispatch,
    request: &EmbeddingRequest,
    context: &CallContext<'_, Metadata, Client>,
) -> Result<EmbeddingResult, DispatchError>
where
    Metadata: Sync,
    Client: Sync,
{
    let deployment = dispatch.bounded_deployment(&request.model_id, &request.call_id, context)?;
    // LiteLLM builds a rule input for chat messages only, so an embedding request never reaches a rule
    let model = request
        .request
        .model
        .as_deref()
        .unwrap_or(&request.model_id);
    dispatch.evaluate_rules(None, model)?;
    let call = EmbeddingCallInput {
        deployment,
        request: &request.request,
    };
    request.calls.embed(call).await.map_err(DispatchError::from)
}

mod tests;
