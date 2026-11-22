use std::collections::BTreeMap;
use std::error::Error;

use betterlite_auth::current_spend::{BUDGET_UNVERIFIABLE_DETAIL, CurrentSpendError};
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};

mod tests;

// user_api_key_auth's handler: the strict HTTPException keeps its 503, any other raise becomes a 401 auth_error
pub fn spend_failure_exception<CacheError>(error: &CurrentSpendError<CacheError>) -> ProxyException
where
    CacheError: Error,
{
    let (message, status) = match error {
        CurrentSpendError::Unverifiable { .. } => (BUDGET_UNVERIFIABLE_DETAIL.to_owned(), 503),
        CurrentSpendError::Cache(_)
        | CurrentSpendError::Value(_)
        | CurrentSpendError::Batch(_)
        | CurrentSpendError::Reseed(_) => (format!("Authentication Error, {error}"), 401),
    };
    ProxyException::new(ProxyExceptionInit {
        message,
        error_type: "auth_error".to_owned(),
        param: Some("None".to_owned()),
        code: ExceptionCode::Status(status),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}
