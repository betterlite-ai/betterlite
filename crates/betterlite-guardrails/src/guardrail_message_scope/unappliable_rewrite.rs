mod tests;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "Guardrail '{guardrail_name}' rewrote the request in a way this endpoint cannot apply, so the request was rejected rather than sent unrewritten"
)]
pub struct UnappliableRequestRewrite {
    pub guardrail_name: String,
}

impl UnappliableRequestRewrite {
    #[must_use]
    pub fn new(guardrail_name: Option<&str>) -> Self {
        Self {
            guardrail_name: guardrail_name.unwrap_or("unknown").to_owned(),
        }
    }
}
