use std::error::Error;

mod tests;

// LiteLLM's created task can fail where nobody awaits it; here the failure is said out loud and the relay goes on
pub(super) fn report_flush_outcome<E>(outcome: Result<(), E>, chunk_count: usize)
where
    E: Error,
{
    if let Err(error) = outcome {
        tracing::error!(
            chunks = chunk_count,
            %error,
            "the passthrough spend-tracking flush failed; {chunk_count} collected chunks dropped"
        );
    }
}
