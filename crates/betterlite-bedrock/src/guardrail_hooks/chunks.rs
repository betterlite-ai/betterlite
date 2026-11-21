use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

#[derive(Debug, Clone, PartialEq)]
pub enum StreamChunk {
    Json(GuardrailValue),
    Bytes(Vec<u8>),
}
