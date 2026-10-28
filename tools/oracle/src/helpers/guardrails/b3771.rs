use std::collections::BTreeMap;

use betterlite_guardrails::generic_guardrail_api::extract_user_api_key_metadata;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::grayswan::wire::GraySwanResponse;
use betterlite_guardrails::grayswan::{GraySwanGuardrail, GraySwanSettings, OnFlaggedAction};
use secrecy::SecretString;
use serde_json::json;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        if input.get("metadata").is_some() || input.get("litellm_metadata").is_some() {
            metadata(input)
        } else if input.get("score").is_some() || input.get("action").is_some() {
            verdict(input)
        } else {
            Self::not_built("b3771")
        }
    }
}

fn metadata(input: &serde_json::Value) -> crate::helpers::HelperResult {
    let litellm_json = input
        .get("litellm_metadata")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let metadata_json = input.get("metadata").cloned().unwrap_or_else(|| json!({}));
    let litellm_values = serde_json::from_value::<BTreeMap<String, Option<String>>>(litellm_json)
        .map_err(|error| failure(error.to_string()))?;
    let metadata_values = serde_json::from_value::<BTreeMap<String, Option<String>>>(metadata_json)
        .map_err(|error| failure(error.to_string()))?;
    let merged = extract_user_api_key_metadata(Some(&litellm_values), Some(&metadata_values));
    let serialized = serde_json::to_value(merged).map_err(|error| failure(error.to_string()))?;
    let object = serialized.as_object().cloned().unwrap_or_default();
    Ok(serde_json::Value::Object(
        object
            .into_iter()
            .filter(|(_, item)| !item.is_null())
            .collect(),
    ))
}

fn verdict(input: &serde_json::Value) -> crate::helpers::HelperResult {
    let guardrail = grayswan_guardrail(input);
    let response = GraySwanResponse {
        violation_score: input
            .get("score")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0_f64),
        violation_reason: Some("oracle violation".to_owned()),
        messages: None,
    };
    let value = guardrail
        .process_response(response, GuardrailInputType::Request, Vec::new())
        .map_err(|error| failure(error.to_string()))?;
    serde_json::to_value(json!({"allowed": true, "flagged": value.flagged}))
        .map_err(|error| failure(error.to_string()))
}

fn grayswan_guardrail(input: &serde_json::Value) -> GraySwanGuardrail {
    let action = match input.get("action").and_then(serde_json::Value::as_str) {
        Some("PASSTHROUGH") => OnFlaggedAction::Passthrough,
        Some("MONITOR") => OnFlaggedAction::Monitor,
        _ => OnFlaggedAction::Block,
    };
    GraySwanGuardrail::with_client(
        SecretString::from("oracle-key"),
        Some("http://oracle.invalid".to_owned()),
        GraySwanSettings {
            on_flagged_action: action,
            violation_threshold: 0.5,
            reasoning_mode: None,
            categories: None,
            policy_id: None,
            fail_open: true,
        },
        reqwest::Client::default(),
        betterlite_guardrails::grayswan::DEFAULT_TIMEOUT,
    )
}

fn failure(message: String) -> Box<crate::helpers::HelperError> {
    let error = crate::helpers::HelperError {
        class: "GuardrailError".to_owned(),
        status: Some(400),
        error_type: Some(Some("invalid_request_error".to_owned())),
        param: None,
        code: None,
        message,
    };
    Box::new(error)
}
