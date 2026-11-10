use std::sync::Arc;

use crate::rubrik_delivery::logger::RubrikLogger;

use super::context::{LoggingObject, RequestData};
use super::error::ModifyResponseException;
use super::guarded::guarded;
use super::inputs::{GuardrailInputs, ModerationInputType};
use super::prompt::moderate_prompt;
use super::response::moderate_response;

pub async fn apply_guardrail(
    logger: &RubrikLogger,
    inputs: GuardrailInputs,
    request_data: &mut RequestData,
    input_type: ModerationInputType,
    logging_obj: Option<&Arc<LoggingObject>>,
) -> Result<GuardrailInputs, ModifyResponseException> {
    match input_type {
        ModerationInputType::Request => {
            let operation = moderate_prompt(logger, inputs.clone(), request_data, logging_obj);
            guarded(operation, inputs, "Prompt moderation").await
        }
        ModerationInputType::Response => {
            let operation = moderate_response(logger, inputs.clone(), request_data, logging_obj);
            guarded(operation, inputs, "Response moderation").await
        }
        ModerationInputType::Unrecognized => Ok(inputs),
    }
}
