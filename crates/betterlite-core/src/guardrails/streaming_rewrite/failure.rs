use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error(
    "Guardrail '{guardrail_name}' rewrote the streamed response but the rewrite cannot be written back to the stream: {reason}"
)]
pub struct UndeliverableStreamRewrite {
    pub guardrail_name: String,
    pub reason: String,
}
