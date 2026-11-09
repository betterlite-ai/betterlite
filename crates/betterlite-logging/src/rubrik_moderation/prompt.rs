use std::sync::Arc;

use crate::rubrik::block_decision::extract_prompt_refusal;
use crate::rubrik::moderation_payload::join_texts;
use crate::rubrik::prompt_payload::{PromptModerationInputs, build_prompt_moderation};
use crate::rubrik_delivery::logger::RubrikLogger;
use crate::rubrik_delivery::post::post_prompt_moderation;

use super::context::{LoggingObject, RequestData, prompt_request, stash_block_context};
use super::error::{ModerationError, ModifyResponseException};
use super::inputs::GuardrailInputs;
use super::service::service_response;

pub async fn moderate_prompt(
    logger: &RubrikLogger,
    inputs: GuardrailInputs,
    request_data: &mut RequestData,
    logging_obj: Option<&Arc<LoggingObject>>,
) -> Result<GuardrailInputs, ModerationError> {
    // Text-only requests reach the webhook when their joined texts are non-empty; LiteLLM builds the synthetic message only for this decision.
    if inputs.structured_messages.is_empty() && join_texts(&inputs.texts).is_empty() {
        return Ok(inputs);
    }
    let request = prompt_request(&request_data.fields)?;
    let payload = build_prompt_moderation(
        &PromptModerationInputs {
            model: inputs.model.as_deref(),
            structured_messages: &inputs.structured_messages,
            tools: inputs.tools.clone(),
        },
        &request,
    );
    let fields = post_prompt_moderation(logger, &payload).await?;
    let refusal = extract_prompt_refusal(&service_response(&fields)?);
    let Some(explanation) = refusal else {
        return Ok(inputs);
    };
    let model = inputs
        .model
        .clone()
        .or_else(|| request.model.map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned());
    stash_block_context(logging_obj, request_data);
    Err(ModerationError::Blocked(Box::new(
        ModifyResponseException {
            message: explanation,
            model,
            request_data: Box::new(request_data.clone()),
            guardrail_name: logger.settings.guardrail_name.clone(),
        },
    )))
}
