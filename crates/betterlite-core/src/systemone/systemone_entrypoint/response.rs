use betterlite_models::decisions::systemone::DecisionsResponse;

use super::super::SystemOneEntryError;
use crate::api::decisions_send::format::DecisionsOutput;

mod tests;

// _systemone_response: LiteLLM asserts the shape, and a wrong one is an internal failure rather than a caller error
pub(crate) fn systemone_response(
    output: DecisionsOutput,
) -> Result<DecisionsResponse, SystemOneEntryError> {
    match output {
        DecisionsOutput::SystemOne(response) => Ok(response),
        DecisionsOutput::OpenAi(_response) => Err(SystemOneEntryError::ResponseFormat {
            format: "OpenAI decision",
        }),
    }
}
