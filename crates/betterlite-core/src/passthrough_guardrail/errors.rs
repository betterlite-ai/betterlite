use betterlite_logging::private_json::error::PrivateJsonError;

#[derive(Debug, thiserror::Error)]
pub enum PassthroughPayloadError {
    #[error("the passthrough payload cannot be serialized")]
    Serialize(#[source] PrivateJsonError),
}

#[derive(Debug, thiserror::Error)]
pub enum PassthroughGuardrailError<Failure: 'static> {
    #[error(transparent)]
    Payload(#[from] PassthroughPayloadError),
    #[error(transparent)]
    Guardrail(Failure),
}
