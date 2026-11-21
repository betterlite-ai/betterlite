use betterlite_core::api::responses_compact_call::error::CompactResponsesError;
use betterlite_redact::secret::Redactor;

use crate::sdk_http::result::Failure;

// The compact wrapper's SDK exception is built without a body, so type, param and code stay null
pub(super) fn failure(
    error: &CompactResponsesError,
    provider: &str,
    model: &str,
    redactor: &Redactor,
) -> Failure {
    let projected = error.sdk_failure(provider, model, redactor);
    Failure {
        class: projected.class.to_owned(),
        status: projected.status,
        kind: Some(projected.kind),
        param: Some(projected.param),
        code: Some(projected.code),
        message: projected.message,
    }
}
