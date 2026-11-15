use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::crowdstrike_aidr::error::CrowdStrikeError;
use betterlite_guardrails::crowdstrike_aidr::handler::{ConstructorArgs, Dependencies};
use betterlite_guardrails::crowdstrike_streaming::{StreamingParams, StreamingSettings};
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::guardrail_records::error::GuardrailSpanError;
use betterlite_guardrails::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailSpanSink,
};
use serde_json::{Value, json};

use super::SettingsFields;
use crate::helpers::HelperError;
use crate::mock::MockProvider;

pub(super) fn fail(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ConfigError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(super) fn constructor_args(settings: &SettingsFields) -> ConstructorArgs {
    ConstructorArgs {
        guardrail_name: settings.guardrail_name.clone(),
        api_key: settings.api_key.clone(),
        api_base: settings.api_base.clone(),
        fail_on_error: settings.fail_on_error,
        streaming: StreamingParams {
            buffer_until_moderated: settings.buffer_until_moderated,
            buffer_release_on_scan: settings.buffer_release_on_scan,
            end_of_stream_only: settings.end_of_stream_only,
            sampling_rate: settings.sampling_rate,
        },
        timeout: settings.timeout,
    }
}

pub(super) struct DiscardSink;

impl GuardrailSpanSink for DiscardSink {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

pub(super) fn dependencies() -> Dependencies {
    Dependencies {
        client: reqwest::Client::default(),
        sink: Box::new(DiscardSink),
        now: std::time::SystemTime::now,
    }
}

pub(super) fn input_type(name: &str) -> GuardrailInputType {
    if name == "response" {
        GuardrailInputType::Response
    } else {
        GuardrailInputType::Request
    }
}

pub(super) fn streaming_json(settings: &StreamingSettings) -> Value {
    json!({
        "buffer_until_moderated": settings.buffer_until_moderated,
        "buffer_release_on_scan": settings.buffer_release_on_scan,
        "end_of_stream_only": settings.end_of_stream_only,
        "sampling_rate": settings.sampling_rate,
    })
}

pub(super) fn projection(output: &GuardrailApiInputs) -> Value {
    json!({
        "texts": output.texts,
        "tools": output.tools,
        "tool_calls": output.tool_calls,
        "structured_messages": output.structured_messages,
    })
}

pub(super) fn outcome(error: &CrowdStrikeError) -> Value {
    match error {
        CrowdStrikeError::Blocked { .. } | CrowdStrikeError::MalformedTransformed { .. } => {
            json!({"tag": "http_error", "status": error.status_code().as_u16()})
        }
        CrowdStrikeError::ResponseJson { .. }
        | CrowdStrikeError::JsonEncode { .. }
        | CrowdStrikeError::InvalidParam { .. } => json!({"tag": "parse_error"}),
        CrowdStrikeError::MissingEnvironment { .. }
        | CrowdStrikeError::Timeout { .. }
        | CrowdStrikeError::Request { .. }
        | CrowdStrikeError::InvalidHeader { .. }
        | CrowdStrikeError::HttpStatus { .. }
        | CrowdStrikeError::ResponseTooLarge { .. }
        | CrowdStrikeError::Record { .. } => json!({"tag": "internal_error"}),
    }
}

pub(super) fn applied_guardrails(request_data: &BTreeMap<String, GuardrailValue>) -> Vec<String> {
    let key = if request_data.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    };
    let Some(GuardrailValue::Object(bucket)) = request_data.get(key) else {
        return Vec::new();
    };
    let Some(GuardrailValue::Array(items)) = bucket.get("applied_guardrails") else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match item {
            GuardrailValue::Text(name) => Some(name.clone()),
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Array(_)
            | GuardrailValue::Object(_) => None,
        })
        .collect()
}

pub(super) fn recorded_call(
    mock: Option<MockProvider>,
) -> Result<(Option<String>, Option<Value>), Box<HelperError>> {
    let calls = mock
        .map(MockProvider::finish)
        .transpose()
        .map_err(|error| fail(error.to_string()))?;
    let recorded = calls.unwrap_or_default().into_iter().next();
    let path = recorded.as_ref().map(|call| call.request.path.clone());
    let body = recorded.map(|call| call.request.body);
    Ok((path, body))
}
