use betterlite_logging::private_json::error::PrivateJsonError;

#[derive(Debug, thiserror::Error)]
pub enum PassthroughPayloadError {
    #[error("the passthrough payload cannot be serialized")]
    Serialize(#[source] PrivateJsonError),
    #[error("the passthrough payload is not a JSON object")]
    Parse(#[source] serde_json::Error),
    #[error("the passthrough payload holds the integer {text}, which fits neither i64 nor u64")]
    IntegerOutOfRange { text: String },
}

#[derive(Debug, thiserror::Error)]
pub enum PassthroughGuardrailError<Failure: 'static> {
    #[error(transparent)]
    Payload(#[from] PassthroughPayloadError),
    #[error(transparent)]
    Guardrail(Failure),
}
