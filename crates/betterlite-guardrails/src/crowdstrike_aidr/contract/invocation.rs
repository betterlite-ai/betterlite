use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::crowdstrike_aidr::api::call_guard;
use crate::crowdstrike_aidr::error::{CrowdStrikeError, error_response};
use crate::crowdstrike_aidr::handler::CrowdStrikeAidr;
use crate::crowdstrike_aidr::models::{EventType, GuardInput, GuardPayload, GuardResult};
use crate::crowdstrike_normalization::merge_metadata_bags;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::guardrail_records::metadata::add_guardrail_record;
use crate::guardrail_records::record::{
    GuardrailRecordContext, GuardrailRecordInput, GuardrailStatus,
};

const USER_ID_KEY: &str = "user_api_key_user_id";
const USER_EMAIL_KEY: &str = "user_api_key_user_email";

pub(super) fn build_payload(
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

pub(super) fn present(
    metadata: &BTreeMap<String, GuardrailValue>,
    key: &str,
) -> Option<GuardrailValue> {
    metadata
        .get(key)
        .filter(|value| **value != GuardrailValue::Null)
        .cloned()
}

// LiteLLM's _call_or_fail_open: a blocked verdict and its malformed twin always reach the caller
pub(super) async fn call_or_fail_open(
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
pub(super) struct Timing {
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

pub(super) fn record_failure(
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

pub(super) fn epoch_seconds(time: SystemTime) -> f64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

// LiteLLM's add_guardrail_to_applied_guardrails_header, run once the API call returned or failed open
pub(super) fn record_applied(
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

pub(super) fn metadata_key(request_data: &BTreeMap<String, GuardrailValue>) -> &'static str {
    if request_data.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    }
}

pub(super) fn take_bucket(
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

pub(super) fn applied_names(bucket: &BTreeMap<String, GuardrailValue>) -> Vec<String> {
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

pub(super) fn failure(error: &CrowdStrikeError) -> GuardrailFailure {
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
