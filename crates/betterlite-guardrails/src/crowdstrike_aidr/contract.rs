use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::crowdstrike_aidr::api::call_guard;
use crate::crowdstrike_aidr::error::{CrowdStrikeError, error_response};
use crate::crowdstrike_aidr::handler::{
    CrowdStrikeAidr, CrowdStrikeAidrConfig, SUPPORTED_EVENT_HOOKS,
};
use crate::crowdstrike_aidr::models::{EventType, GuardInput, GuardPayload, GuardResult};
use crate::crowdstrike_message_selection::{
    build_guard_input_for_request, build_guard_input_for_response, extract_transformed_texts,
};
use crate::crowdstrike_normalization::merge_metadata_bags;
use crate::crowdstrike_writeback::{merge_request_transforms, writeback_messages};
use crate::generic_guardrail_contracts::{
    GuardrailExtraValue, GuardrailInputType, GuardrailMessage, GuardrailToolParam,
};
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::guardrail_records::metadata::add_guardrail_record;
use crate::guardrail_records::record::{
    GuardrailRecordContext, GuardrailRecordInput, GuardrailStatus,
};

const USER_ID_KEY: &str = "user_api_key_user_id";
const USER_EMAIL_KEY: &str = "user_api_key_user_email";

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

fn build_payload(
    inputs: &GuardrailApiInputs,
    guard_input: GuardInput,
    event_type: EventType,
    request_data: &BTreeMap<String, GuardrailValue>,
) -> GuardPayload {
    let metadata = merge_metadata_bags(request_data);
    let (user_id, extra_info) = match metadata {
        Some(metadata) => {
            let mut extra = BTreeMap::new();
            if let Some(email) = present(&metadata, USER_EMAIL_KEY) {
                extra.insert("user_name".to_owned(), email);
            }
            (present(&metadata, USER_ID_KEY), Some(extra))
        }
        None => (None, None),
    };
    GuardPayload {
        guard_input,
        event_type,
        model: inputs.model.clone().filter(|model| !model.is_empty()),
        user_id,
        extra_info,
    }
}

fn present(metadata: &BTreeMap<String, GuardrailValue>, key: &str) -> Option<GuardrailValue> {
    metadata
        .get(key)
        .filter(|value| **value != GuardrailValue::Null)
        .cloned()
}

// LiteLLM's _call_or_fail_open: a blocked verdict and its malformed twin always reach the caller
async fn call_or_fail_open(
    guardrail: &CrowdStrikeAidr,
    payload: &GuardPayload,
    hook_name: &str,
    request_data: &mut BTreeMap<String, GuardrailValue>,
    logging_obj: Option<&mut GuardrailLogTarget>,
) -> Result<GuardResult, CrowdStrikeError> {
    let start_time = (guardrail.now)();
    match call_guard(guardrail, payload, hook_name).await {
        Ok(result) => Ok(result),
        Err(
            error @ (CrowdStrikeError::Blocked { .. }
            | CrowdStrikeError::MalformedTransformed { .. }),
        ) => Err(error),
        Err(error) => {
            if guardrail.settings.fail_on_error {
                return Err(error);
            }
            tracing::error!(
                "CrowdStrike AIDR Guardrail failed open | hook_name: {hook_name} error: {error}"
            );
            let timing = Timing::between(start_time, (guardrail.now)());
            record_failure(guardrail, request_data, logging_obj, &error, timing)?;
            Ok(GuardResult::default())
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Timing {
    start_time: f64,
    end_time: f64,
    duration: f64,
}

impl Timing {
    fn between(start: SystemTime, end: SystemTime) -> Self {
        Self {
            start_time: epoch_seconds(start),
            end_time: epoch_seconds(end),
            duration: end.duration_since(start).unwrap_or_default().as_secs_f64(),
        }
    }
}

fn record_failure(
    guardrail: &CrowdStrikeAidr,
    request_data: &mut BTreeMap<String, GuardrailValue>,
    logging_obj: Option<&mut GuardrailLogTarget>,
    error: &CrowdStrikeError,
    timing: Timing,
) -> Result<(), CrowdStrikeError> {
    let mut context = GuardrailRecordContext {
        base: &guardrail.base,
        request_data,
        log_target: logging_obj,
        sink: guardrail.sink.as_ref(),
    };
    add_guardrail_record(
        &mut context,
        GuardrailRecordInput {
            guardrail_response: error_response(error),
            guardrail_status: GuardrailStatus::GuardrailFailedToRespond,
            start_time: Some(timing.start_time),
            end_time: Some(timing.end_time),
            duration: Some(timing.duration),
            masked_entity_count: None,
            guardrail_provider: None,
            event_type: None,
            tracing_detail: None,
        },
    )?;
    Ok(())
}

fn epoch_seconds(time: SystemTime) -> f64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

// LiteLLM's add_guardrail_to_applied_guardrails_header, run once the API call returned or failed open
fn record_applied(
    guardrail: &CrowdStrikeAidr,
    request_data: &mut BTreeMap<String, GuardrailValue>,
) {
    let Some(name) = guardrail.settings.guardrail_name.as_deref() else {
        return;
    };
    if !request_data.contains_key("body") && !request_data.contains_key("messages") {
        return;
    }
    let key = metadata_key(request_data);
    let mut bucket = take_bucket(request_data, key);
    let mut applied = applied_names(&bucket);
    if !applied.iter().any(|entry| entry == name) {
        applied.push(name.to_owned());
    }
    bucket.insert(
        "applied_guardrails".to_owned(),
        GuardrailValue::Array(applied.into_iter().map(GuardrailValue::Text).collect()),
    );
    request_data.insert(key.to_owned(), GuardrailValue::Object(bucket));
}

fn metadata_key(request_data: &BTreeMap<String, GuardrailValue>) -> &'static str {
    if request_data.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    }
}

fn take_bucket(
    request_data: &mut BTreeMap<String, GuardrailValue>,
    key: &str,
) -> BTreeMap<String, GuardrailValue> {
    match request_data.remove(key) {
        Some(GuardrailValue::Object(fields)) => fields,
        None
        | Some(
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Text(_)
            | GuardrailValue::Array(_),
        ) => BTreeMap::new(),
    }
}

fn applied_names(bucket: &BTreeMap<String, GuardrailValue>) -> Vec<String> {
    match bucket.get("applied_guardrails") {
        Some(GuardrailValue::Array(items)) => items
            .iter()
            .filter_map(|item| match item {
                GuardrailValue::Text(entry) => Some(entry.clone()),
                GuardrailValue::Null
                | GuardrailValue::Boolean(_)
                | GuardrailValue::SignedInteger(_)
                | GuardrailValue::UnsignedInteger(_)
                | GuardrailValue::Decimal(_)
                | GuardrailValue::Array(_)
                | GuardrailValue::Object(_) => None,
            })
            .collect(),
        None
        | Some(
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Text(_)
            | GuardrailValue::Object(_),
        ) => Vec::new(),
    }
}

fn failure(error: &CrowdStrikeError) -> GuardrailFailure {
    match error {
        CrowdStrikeError::Blocked { .. } | CrowdStrikeError::MalformedTransformed { .. } => {
            GuardrailFailure::HttpObject {
                status: error.status_code().as_u16(),
                detail: error.detail(),
            }
        }
        other => GuardrailFailure::Http {
            status: other.status_code().as_u16(),
            detail: other.to_string(),
        },
    }
}

mod tests;
