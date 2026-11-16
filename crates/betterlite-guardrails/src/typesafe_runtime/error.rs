use std::time::TryFromFloatSecsError;

use indexmap::IndexMap;
use thiserror::Error;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::typesafe::typesafe_config::TypeSafeConfigError;
use crate::typesafe::typesafe_failure_handling::TypeSafeError;

#[derive(Debug, Error)]
pub enum TypeSafeRuntimeError {
    #[error(
        "TypeSafe guardrail requires an API key. Set `api_key` in the guardrail config or the TYPESAFE_API_KEY env var."
    )]
    MissingApiKey,
    #[error("the guardrail timeout {seconds} is not a valid duration")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error(transparent)]
    Settings(#[from] TypeSafeConfigError),
}

// LiteLLM raises HTTPException(status_code=502, detail={"error": message}) on the fail_closed path
impl From<TypeSafeError> for GuardrailFailure {
    fn from(error: TypeSafeError) -> Self {
        let status = error.status_code().as_u16();
        let TypeSafeError::Unavailable { message } = error;
        Self::HttpObject {
            status,
            detail: IndexMap::from([("error".to_owned(), GuardrailValue::Text(message))]),
        }
    }
}
