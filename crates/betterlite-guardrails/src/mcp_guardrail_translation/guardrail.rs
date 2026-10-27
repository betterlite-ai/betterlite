use std::error::Error as StdError;
use std::future::Future;

use crate::generic_guardrail_contracts::{GuardrailInputType, GuardrailToolParam};

#[derive(Debug, Clone, PartialEq)]
pub struct GuardrailInputs {
    pub tools: Vec<GuardrailToolParam>,
    pub texts: Vec<String>,
}

pub trait ApplyGuardrail {
    type Error: StdError + 'static;

    fn apply_guardrail(
        &self,
        inputs: GuardrailInputs,
        input_type: GuardrailInputType,
    ) -> impl Future<Output = Result<Option<Vec<String>>, Self::Error>> + Send;
}
