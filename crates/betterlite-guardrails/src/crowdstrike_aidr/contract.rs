use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::crowdstrike_aidr::error::CrowdStrikeError;
use crate::crowdstrike_aidr::handler::{
    CrowdStrikeAidr, CrowdStrikeAidrConfig, SUPPORTED_EVENT_HOOKS,
};

use self::invocation::{build_payload, call_or_fail_open, failure, record_applied};
use crate::crowdstrike_aidr::models::{EventType, GuardInput, GuardResult};
use crate::crowdstrike_message_selection::{
    build_guard_input_for_request, build_guard_input_for_response, extract_transformed_texts,
};
use crate::crowdstrike_writeback::{merge_request_transforms, writeback_messages};
use crate::generic_guardrail_contracts::{
    GuardrailExtraValue, GuardrailInputType, GuardrailMessage, GuardrailToolParam,
};
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};

impl GuardrailContract for CrowdStrikeAidr {
    async fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        run_guardrail(self, inputs, request_data, input_type, logging_obj)
            .await
            .map_err(|error| failure(&error))
    }

    fn get_config_model(&self) -> Option<GuardrailValue> {
        Some(GuardrailValue::Object(BTreeMap::from([
            (
                "model".to_owned(),
                GuardrailValue::Text("CrowdStrikeAidrConfig".to_owned()),
            ),
            (
                "ui_friendly_name".to_owned(),
                GuardrailValue::Text(CrowdStrikeAidrConfig::ui_friendly_name().to_owned()),
            ),
        ])))
    }

    fn get_supported_event_hooks(&self) -> Option<Vec<GuardrailEventHooks>> {
        Some(SUPPORTED_EVENT_HOOKS.to_vec())
    }

    fn messages_cover_full_request(&self) -> bool {
        self.settings.skip.system || self.settings.skip.tool
    }
}

// LiteLLM's apply_guardrail, with the built payload handed to the API call
pub async fn run_guardrail(
    guardrail: &CrowdStrikeAidr,
    inputs: GuardrailApiInputs,
    request_data: &mut BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
    logging_obj: Option<&mut GuardrailLogTarget>,
) -> Result<GuardrailApiInputs, CrowdStrikeError> {
    let Some((guard_input, sent_indices, event_type, hook_name)) =
        select_input(&inputs, input_type)?
    else {
        return Ok(inputs);
    };
    let payload = build_payload(&inputs, guard_input, event_type, request_data);
    let result =
        call_or_fail_open(guardrail, &payload, hook_name, request_data, logging_obj).await?;
    record_applied(guardrail, request_data);
    let Some(guard_output) = transformed_output(&result) else {
        return Ok(inputs);
    };
    let texts = inputs.texts.clone().unwrap_or_default();
    Ok(reassembled(
        guardrail,
        &guard_output,
        &Transformed {
            request_data,
            structured: inputs.structured_messages.as_deref(),
            texts: &texts,
            sent_indices: &sent_indices,
            tools: inputs.tools.clone(),
            tool_calls: inputs.tool_calls.clone(),
            input_type,
        },
    ))
}

fn select_input(
    inputs: &GuardrailApiInputs,
    input_type: GuardrailInputType,
) -> Result<Option<(GuardInput, Vec<usize>, EventType, &'static str)>, CrowdStrikeError> {
    match input_type {
        GuardrailInputType::Request => {
            let Some(with_indices) = build_guard_input_for_request(inputs)? else {
                return Ok(None);
            };
            Ok(Some((
                with_indices.guard_input,
                with_indices.sent_indices,
                EventType::Input,
                "apply_guardrail (request)",
            )))
        }
        GuardrailInputType::Response => {
            let guard_input = build_guard_input_for_response(inputs);
            if guard_input.messages.is_empty() {
                return Ok(None);
            }
            Ok(Some((
                guard_input,
                Vec::new(),
                EventType::Output,
                "apply_guardrail (response)",
            )))
        }
    }
}

fn transformed_output(result: &GuardResult) -> Option<GuardInput> {
    if result.transformed.unwrap_or(false) {
        result.guard_output.clone()
    } else {
        None
    }
}

struct Transformed<'call> {
    request_data: &'call BTreeMap<String, GuardrailValue>,
    structured: Option<&'call [GuardrailMessage]>,
    texts: &'call [String],
    sent_indices: &'call [usize],
    tools: Option<Vec<GuardrailToolParam>>,
    tool_calls: Option<Vec<GuardrailExtraValue>>,
    input_type: GuardrailInputType,
}

// LiteLLM's apply_guardrail return value: only texts, tools, tool_calls and structured messages survive
fn reassembled(
    guardrail: &CrowdStrikeAidr,
    guard_output: &GuardInput,
    transformed: &Transformed<'_>,
) -> GuardrailApiInputs {
    let mut result = empty_result(collect_texts(guard_output, transformed));
    if let Some(tools) = transformed.tools.clone().filter(|tools| !tools.is_empty()) {
        result.tools = Some(tools);
    }
    if let Some(tool_calls) = transformed
        .tool_calls
        .clone()
        .filter(|calls| !calls.is_empty())
    {
        result.tool_calls = Some(tool_calls);
    }
    carry_structured(&mut result, guardrail, guard_output, transformed);
    result
}

fn collect_texts(guard_output: &GuardInput, transformed: &Transformed<'_>) -> Vec<String> {
    match transformed.input_type {
        GuardrailInputType::Request => merge_request_transforms(
            guard_output,
            transformed.structured,
            transformed.texts,
            transformed.sent_indices,
        ),
        GuardrailInputType::Response => {
            extract_transformed_texts(guard_output, transformed.texts.len())
        }
    }
}

fn empty_result(texts: Vec<String>) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: Some(texts),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    }
}

fn carry_structured(
    result: &mut GuardrailApiInputs,
    guardrail: &CrowdStrikeAidr,
    guard_output: &GuardInput,
    transformed: &Transformed<'_>,
) {
    let Some(structured) = transformed
        .structured
        .filter(|messages| !messages.is_empty())
    else {
        return;
    };
    let rebuilt = match transformed.input_type {
        GuardrailInputType::Request => writeback_messages(
            structured,
            guard_output,
            transformed.sent_indices,
            transformed.request_data,
            guardrail.settings.skip,
        ),
        GuardrailInputType::Response => None,
    };
    result.structured_messages = Some(rebuilt.unwrap_or_else(|| structured.to_vec()));
}

mod invocation;
mod tests;
