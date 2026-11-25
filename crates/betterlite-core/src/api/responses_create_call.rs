pub mod deps;
pub mod error;
mod execute;
mod hook;
mod prepare;
mod tests;

use crate::api::responses_create::bridge::Created;
use crate::api::responses_create::call::ResponsesCreateCall;
use crate::api::responses_create_http::service::LogContext;

use self::deps::{AsyncCreateDeps, CreateReturn, SyncHandoff};
use self::error::{AsyncCreateCause, AsyncCreateError};

// LiteLLM's aresponses: it converts the arguments, runs the async prompt hook and awaits the sync answer once
pub async fn aresponses<Context, Handoff>(
    deps: &AsyncCreateDeps<'_>,
    execution: &Handoff,
    call: &ResponsesCreateCall,
    context: Context,
    log: &mut LogContext<'_, '_>,
) -> Result<Created<Context>, AsyncCreateError>
where
    Context: Send,
    Handoff: SyncHandoff<Context>,
{
    let prepared = prepare::prepare(deps, call).await?;
    let returned = execution.handoff(&prepared.call, context, log);
    let awaited = match returned {
        CreateReturn::Completed(outcome) => Ok(outcome),
        CreateReturn::Pending(pending) => pending.await,
    };
    let outcome = awaited.map_err(|error| AsyncCreateError {
        cause: Box::new(AsyncCreateCause::Create(error)),
        provider: prepared.provider.clone(),
        model: prepared.model.clone(),
    })?;
    match outcome {
        Some(created) => execute::identity(created, &prepared),
        None => Err(AsyncCreateError {
            cause: Box::new(AsyncCreateCause::NullResponse),
            provider: prepared.provider,
            model: prepared.model,
        }),
    }
}
