#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SelectionError {
    #[error("guardrail '{guardrail_name}' named in the request metadata is not configured")]
    UnknownGuardrail { guardrail_name: String },
}
