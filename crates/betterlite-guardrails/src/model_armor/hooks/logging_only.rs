use std::error::Error as StdError;

use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::guardrail_selection::gate::GuardrailSelection as _;
use crate::model_armor::decisions::{
    build_block_error_detail, build_logging_response, should_block_content,
};
use crate::model_armor::transport::ArmorResponse;
use crate::model_armor::transport::error::{ArmorCallError, TransportError};
use crate::model_armor::transport::tokens::ArmorTokens;
use crate::model_armor::transport::wire::Source;

use super::error_handling::HookError;
use super::logging_information::{self, ApplyVerdict};
use super::timing::{self, RecordTimes};
use super::{Data, HookContext, ModelArmorHooks};

impl<T: ArmorTokens> ModelArmorHooks<'_, T> {
    // LiteLLM's apply_guardrail: scan, record the verdict, and block only outside logging_only mode
    pub async fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        data: &mut Data,
        input_type: GuardrailInputType,
        context: &mut HookContext<'_>,
    ) -> Result<GuardrailApiInputs, HookError<T::Error>> {
        let text = joined_texts(&inputs);
        if text.is_empty() {
            self.process_response(data, context)?;
            return Ok(inputs);
        }
        let started_at = (self.clock)();
        let result = self
            .transport
            .make_model_armor_request(Some(&text), source(input_type), None)
            .await;
        let times = timing::between(started_at, (self.clock)());
        self.record_verdict(data, context, result, times, input_type)?;
        Ok(inputs)
    }

    fn record_verdict(
        &self,
        data: &mut Data,
        context: &mut HookContext<'_>,
        result: Result<ArmorResponse, ArmorCallError<T::Error>>,
        times: RecordTimes,
        input_type: GuardrailInputType,
    ) -> Result<(), HookError<T::Error>> {
        let armor = match result {
            Ok(armor) => armor,
            Err(error) => {
                let verdict = failure_verdict(error)?;
                return logging_information::record_apply_guardrail(
                    self, data, context, verdict, times,
                );
            }
        };
        let flagged = should_block_content(&armor.verdict, false);
        let response = build_logging_response(&armor.body, self.guardrail.sanitize_error_detail);
        let verdict = if flagged {
            ApplyVerdict::Flagged { response }
        } else {
            ApplyVerdict::Success { response }
        };
        logging_information::record_apply_guardrail(self, data, context, verdict, times)?;
        if flagged
            && !self
                .guardrail
                .base
                .event_hook_is_event_type(GuardrailEventHooks::LoggingOnly)
        {
            return Err(HookError::Blocked {
                status: 400,
                detail: build_block_error_detail(
                    block_message(input_type),
                    &armor.body,
                    self.guardrail.sanitize_error_detail,
                ),
            });
        }
        Ok(())
    }
}

// LiteLLM joins the non-empty texts it is handed, one per line
fn joined_texts(inputs: &GuardrailApiInputs) -> String {
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

const fn source(input_type: GuardrailInputType) -> Source {
    match input_type {
        GuardrailInputType::Request => Source::UserPrompt,
        GuardrailInputType::Response => Source::ModelResponse,
    }
}

const fn block_message(input_type: GuardrailInputType) -> &'static str {
    match input_type {
        GuardrailInputType::Request => "Content blocked by Model Armor",
        GuardrailInputType::Response => "Response blocked by Model Armor",
    }
}

// LiteLLM's `except (ModelArmorAPIError, httpx.HTTPError)`: only those record and let the request run
fn failure_verdict<E>(error: ArmorCallError<E>) -> Result<ApplyVerdict, HookError<E>>
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
