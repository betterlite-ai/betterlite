use std::sync::Arc;

use betterlite_context::call_context::CallContext;

use super::Dispatch;
use super::error::DispatchError;
use super::log::CallOutcome;
use crate::operation::Entry;
use crate::operation::moderation::{
    ModerationCallInput, ModerationFailure, ModerationRequest, ModerationResult,
};

pub async fn execute<Metadata, Client>(
    request: ModerationRequest,
    context: &CallContext<'_, Metadata, Client>,
) -> Result<ModerationResult, DispatchError>
where
    Metadata: Sync,
    Client: Sync,
{
    let dispatch = Arc::clone(&request.dispatch);
    let terminal = dispatch.terminal(Entry::Moderation, &request.call_id, context);
    let result = moderate(&dispatch, &request, context).await;
    let outcome = if result.is_ok() {
        CallOutcome::Succeeded
    } else {
        CallOutcome::Failed
    };
    terminal.record(outcome, result.as_ref().err(), None);
    result
}

async fn moderate<Metadata, Client>(
    dispatch: &Dispatch,
    request: &ModerationRequest,
    context: &CallContext<'_, Metadata, Client>,
) -> Result<ModerationResult, DispatchError>
where
    Metadata: Sync,
    Client: Sync,
{
    let deployment = dispatch.bounded_deployment(&request.model_id, &request.call_id, context)?;
    // moderation guardrails arrive with their later task, so no rule sees this request yet
    dispatch.evaluate_rules(None, &request.model_id)?;
    let call = ModerationCallInput {
        deployment,
        request: &request.request,
    };
    request.calls.moderate(call).await.map_err(rejected)
}

// the shared Rejected variant types error_type as text, so a null type travels to the route as an empty string
fn rejected(failure: ModerationFailure) -> DispatchError {
    DispatchError::Rejected {
        status: failure.status,
        error_type: failure.error_type.unwrap_or_default(),
        param: failure.param,
        code: failure.code.unwrap_or_else(|| failure.status.to_string()),
        message: failure.message,
    }
}

mod tests;
