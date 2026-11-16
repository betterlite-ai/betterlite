use std::error::Error as StdError;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::model_armor::transport::error::{ArmorCallError, TransportError};
use crate::model_armor::transport::wire::Source;

use super::error_handling::HookError;
use super::logging_information::ApplyVerdict;

// LiteLLM joins the non-empty texts it is handed, one per line
pub(super) fn joined_texts(inputs: &GuardrailApiInputs) -> String {
    let mut content = String::new();
    for text in inputs.texts.as_deref().unwrap_or_default() {
        if text.is_empty() {
            continue;
        }
        if !content.is_empty() {
            content.push('\n');
        }
        content.push_str(text);
    }
    content
}

pub(super) const fn source(input_type: GuardrailInputType) -> Source {
    match input_type {
        GuardrailInputType::Request => Source::UserPrompt,
        GuardrailInputType::Response => Source::ModelResponse,
    }
}

pub(super) const fn block_message(input_type: GuardrailInputType) -> &'static str {
    match input_type {
        GuardrailInputType::Request => "Content blocked by Model Armor",
        GuardrailInputType::Response => "Response blocked by Model Armor",
    }
}

// LiteLLM's `except (ModelArmorAPIError, httpx.HTTPError)`: only those record and let the request run
pub(super) fn failure_verdict<E>(error: ArmorCallError<E>) -> Result<ApplyVerdict, HookError<E>>
where
    E: StdError + Send + Sync + 'static,
{
    match error {
        ArmorCallError::Api(api) => Ok(ApplyVerdict::FailedToRespond { detail: api.detail }),
        ArmorCallError::Transport(failure)
            if matches!(
                failure,
                TransportError::Request(_) | TransportError::Timeout(_) | TransportError::Body(_)
            ) =>
        {
            Ok(ApplyVerdict::FailedToRespond {
                detail: failure.to_string(),
            })
        }
        ArmorCallError::Transport(failure) => Err(HookError::Transport(failure)),
    }
}

mod tests;
