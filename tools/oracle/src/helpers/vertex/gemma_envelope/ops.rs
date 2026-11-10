use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::chat_content::{ChatContent, RefuseDownloads};
use betterlite_prompts::arguments::json::Json as PromptJson;
use betterlite_vertex::gemma_envelope::config::VertexGemmaConfig;
use betterlite_vertex::gemma_envelope::error::GemmaEnvelopeError;
use betterlite_vertex::gemma_envelope::request::transform_request;
use betterlite_vertex::gemma_envelope::response::unwrap_predictions;
use serde_json::{Value, json};

use super::input::{FakeStreamCase, Input, TransformCase};
use crate::helpers::HelperError;

pub(super) fn run(input: &Input) -> Result<Value, Box<HelperError>> {
    Ok(json!({
        "wrapper_keys": transform(&input.wrapper_keys)?,
        "context_management_removed": transform(&input.context_management_removed)?,
        "missing_predictions": outcome(&input.missing_predictions),
        "embedded_429": outcome(&input.embedded_429),
        "invalid_error_code": outcome(&input.invalid_error_code),
        "coercible_error_code": outcome(&input.coercible_error_code),
        "fake_stream_flag": fake_streams(&input.fake_stream_flag),
    }))
}

fn transform(case: &TransformCase) -> Result<Value, Box<HelperError>> {
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let content = ChatContent {
        model: &case.model,
        messages: &case.messages,
        optional_params: &case.optional_params,
        provider: case
            .litellm_params
            .get("custom_llm_provider")
            .and_then(PromptJson::as_str),
        api_base: case
            .litellm_params
            .get("api_base")
            .and_then(PromptJson::as_str),
        global_api_base: None,
        openai_system_messages_first: false,
        environment: &environment,
        downloads: &RefuseDownloads,
    };
    let request = transform_request(&content).map_err(|error| envelope_failure(&error))?;
    let (instance_keys, top_keys) = key_lists(&request);
    Ok(json!({
        "request": serde_json::to_value(&request).map_err(|source| encode_failure(&source))?,
        "instance_keys": instance_keys,
        "top_keys": top_keys,
    }))
}

fn key_lists(request: &PromptJson) -> (Vec<String>, Vec<String>) {
    let PromptJson::Object(root) = request else {
        return (Vec::new(), Vec::new());
    };
    let mut top_keys: Vec<String> = root.keys().cloned().collect();
    top_keys.sort_unstable();
    let Some(PromptJson::List(instances)) = root.get("instances") else {
        return (Vec::new(), top_keys);
    };
    let Some(PromptJson::Object(instance)) = instances.first() else {
        return (Vec::new(), top_keys);
    };
    let mut instance_keys: Vec<String> = instance.keys().cloned().collect();
    instance_keys.sort_unstable();
    (instance_keys, top_keys)
}

fn outcome(response: &Json) -> Value {
    match unwrap_predictions(response) {
        Ok(predictions) => json!({"error": Value::Null, "predictions": predictions}),
        Err(error) => json!({
            "error": {"status": error.status_code()},
            "predictions": Value::Null,
        }),
    }
}

fn fake_streams(case: &FakeStreamCase) -> Value {
    let results: Vec<Value> = case
        .cases
        .iter()
        .map(|query| {
            json!({
                "model": query.model,
                "stream": query.stream,
                "provider": query.provider,
                "faked": VertexGemmaConfig::SHOULD_FAKE_STREAM,
            })
        })
        .collect();
    json!({"results": results})
}

fn envelope_failure(error: &GemmaEnvelopeError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("GemmaEnvelopeError"),
        status: Some(error.status_code()),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn encode_failure(source: &serde_json::Error) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("EncodeError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: source.to_string(),
    })
}
