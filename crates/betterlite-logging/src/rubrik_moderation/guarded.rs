use std::future::Future;

use super::error::{ModerationError, ModifyResponseException};
use super::inputs::GuardrailInputs;

// Fail-open: only an intentional policy block escapes; a malformed payload or transient failure returns the inputs unchanged.
pub async fn guarded<Operation>(
    operation: Operation,
    inputs: GuardrailInputs,
    label: &'static str,
) -> Result<GuardrailInputs, ModifyResponseException>
where
    Operation: Future<Output = Result<GuardrailInputs, ModerationError>>,
{
    match operation.await {
        Ok(guarded_inputs) => Ok(guarded_inputs),
        Err(ModerationError::Blocked(exception)) => Err(*exception),
        Err(ModerationError::Malformed(error)) => {
            tracing::error!(
                severity = "critical",
                error = %error,
                "Response moderation service returned a malformed response; requests are not being checked -- verify the webhook configuration. Returning original inputs unchanged."
            );
            Ok(inputs)
        }
        Err(error) => {
            tracing::error!(
                label = %label,
                error = %error,
                "{label} hook failed: {error}. Returning original inputs unchanged."
            );
            Ok(inputs)
        }
    }
}
