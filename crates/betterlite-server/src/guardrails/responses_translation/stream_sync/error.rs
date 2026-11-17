use thiserror::Error;

#[derive(Debug, Error)]
pub enum StreamSyncError {
    #[error(
        "Guardrail '{guardrail_name}' rewrote the streamed response but the rewrite cannot be written back to the stream: {reason}"
    )]
    Undeliverable {
        guardrail_name: String,
        reason: String,
    },
}
