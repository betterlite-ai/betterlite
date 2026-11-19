use std::sync::Arc;

use betterlite_azure::passthrough::config::AzurePassthroughConfig;
use betterlite_azure::passthrough::logging::{
    AzureRelayedLog, NoChatReplyProjection, NoChatStreamAssembly,
};
use betterlite_foundry::chat::sources::NoSources;
use betterlite_foundry::passthrough::auth::{ValidateEnvironmentRequest, validate_environment};
use betterlite_foundry::passthrough::logging::FoundryRelayedLog;
use betterlite_foundry::passthrough::ocr::NativeOcrPassthrough;
use betterlite_foundry::passthrough::plan::{FoundryLogInput, FoundryPassthroughPlan};
use betterlite_foundry::passthrough::url::{
    foundry_root, relay_query_params, without_repeated_native_prefix,
};
use betterlite_ocr::response::OCRResponse;
use betterlite_openai::chat_choices::reply::ResponseReply;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughTarget, PassthroughUrlRequest,
};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use secrecy::SecretString;
use serde_json::{Value, json};

mod input;
mod support;
mod tests;

use input::{
    BaseStep, CollectedStep, Input, NonStreamStep, PrefixStep, QueryStep, RootStep, ValidationStep,
};
use support::{RecordingLogging, capture_warnings, scripted_ocr};

type Plan = FoundryPassthroughPlan;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let sources = NoSources;
        Ok(json!({
            "roots": input.roots.iter().map(root_step).collect::<Vec<Value>>(),
            "prefixes": input.prefixes.iter().map(prefix_step).collect::<Vec<Value>>(),
            "queries": input.queries.iter().map(query_step).collect::<Vec<Value>>(),
            "bases": input.bases.iter().map(base_step).collect::<Vec<Value>>(),
            "validations": input
                .validations
                .iter()
                .map(|step| validation_step(step, &sources))
                .collect::<Vec<Value>>(),
            "nonstreams": input.nonstreams.iter().map(nonstream_step).collect::<Vec<Value>>(),
            "streams": input.streams.iter().map(collected_step).collect::<Vec<Value>>(),
        }))
    }
}

fn plan_for(
    global_api_base: Option<String>,
    environment_api_base: Option<String>,
    litellm_params: Object,
    projector: Option<Arc<dyn NativeOcrPassthrough>>,
) -> Plan {
    let azure = AzurePassthroughConfig {
        api_base_env: environment_api_base,
        api_key_env: None,
        model_call_details: None,
        chat_reply: NoChatReplyProjection,
        chat_stream: NoChatStreamAssembly,
    };
    FoundryPassthroughPlan {
        global_api_base,
        global_api_key: None,
        azure,
        ocr: projector,
        litellm_params,
    }
}

fn target<'call>(model: &'call str, endpoint: &'call str) -> PassthroughTarget<'call> {
    PassthroughTarget {
        model,
        custom_llm_provider: "azure_ai",
        endpoint,
    }
}

fn root_step(step: &RootStep) -> Value {
    match foundry_root(&step.api_base) {
        Ok(root) => json!({"root": root}),
        Err(_) => json!({"error": {"status": null}}),
    }
}

fn prefix_step(step: &PrefixStep) -> Value {
    match without_repeated_native_prefix(&step.root, &step.native_endpoint) {
        Ok(root) => json!({"root": root}),
        Err(_) => json!({"error": {"status": null}}),
    }
}

fn query_step(step: &QueryStep) -> Value {
    let params = relay_query_params(
        step.request_query_params.as_ref(),
        step.deployment_api_version.as_deref(),
        &step.api_base,
    );
    match params {
        Ok(params) => json!({"params": params}),
        Err(_) => json!({"error": {"status": null}}),
    }
}

fn base_step(step: &BaseStep) -> Value {
    let plan = plan_for(
        step.global_api_base.clone(),
        step.environment_api_base.clone(),
        step.litellm_params.clone().unwrap_or_default(),
        None,
    );
    let request = PassthroughUrlRequest {
        api_base: step.api_base.as_deref(),
        api_key: None,
        target: target(&step.model, &step.endpoint),
        request_query_params: step.request_query_params.as_ref(),
        litellm_params: &plan.litellm_params,
    };
    let Ok((url, root)) = plan.get_complete_url(&request) else {
        return json!({"error": {"status": null}});
    };
    json!({"url": url.as_str(), "root": root})
}

fn validation_step(step: &ValidationStep, sources: &NoSources) -> Value {
    let api_key = step
        .api_key
        .as_ref()
        .map(|key| SecretString::from(key.clone()));
    let request = ValidateEnvironmentRequest {
        headers: step.headers.clone(),
        api_key: api_key.as_ref(),
        params: step.params.as_ref(),
        api_base: step.api_base.as_deref(),
        environment: &step.environment,
        refresh_enabled: false,
        sources,
    };
    match validate_environment(request) {
        Ok(headers) => json!({"headers": headers}),
        Err(_) => json!({"error": {"status": null}}),
    }
}

fn outcome_fields(
    outcome: &Option<FoundryRelayedLog>,
) -> (&'static str, Option<(&'static str, Value)>) {
    match outcome {
        Some(FoundryRelayedLog::Azure(AzureRelayedLog::Chat(_))) => ("chat", None),
        Some(FoundryRelayedLog::Azure(AzureRelayedLog::Relay(_))) => ("relay", None),
        Some(FoundryRelayedLog::Azure(AzureRelayedLog::Terminal(_))) => ("terminal", None),
        Some(FoundryRelayedLog::Ocr(response)) => ("ocr", Some(("ocr", ocr_fields(response)))),
        Some(FoundryRelayedLog::Relay(_)) => ("relay", None),
        Some(FoundryRelayedLog::Standard(response)) => (
            "standard",
            serde_json::to_value(&response.response)
                .ok()
                .map(|value| ("standard", value)),
        ),
        None => ("none", None),
    }
}

fn ocr_fields(response: &OCRResponse) -> Value {
    let usage = response.usage_info.as_ref().map(|usage| {
        json!({
            "pages_processed": usage.pages_processed,
            "credits": usage.credits,
            "doc_size_bytes": usage.doc_size_bytes,
        })
    });
    json!({"model": response.model, "pages": response.pages.len(), "usage": usage})
}

fn nonstream_step(step: &NonStreamStep) -> Value {
    let recorder = step.ocr.as_ref().map(scripted_ocr);
    let projector = recorder
        .clone()
        .map(|ocr| -> Arc<dyn NativeOcrPassthrough> { ocr });
    let plan = plan_for(None, None, Object::new(), projector);
    let headers = Headers::new();
    let reply = ResponseReply {
        status: step.status,
        headers: &headers,
        body: step.body.as_bytes(),
    };
    let passthrough_target = target(&step.model, &step.endpoint);
    let request_url = step.request_url.as_deref().unwrap_or(&step.endpoint);
    let request_data = Object::new();
    let input = FoundryLogInput {
        target: &passthrough_target,
        reply: &reply,
        request_data: &request_data,
        request_url,
    };
    let mut logging = RecordingLogging::default();
    let (outcome, warnings) =
        capture_warnings(|| plan.logging_relayed_response(&input, &mut logging));
    let (kind, detail) = outcome_fields(&outcome);
    let calls = recorder
        .as_ref()
        .map_or_else(Vec::new, |ocr| ocr.calls_json());
    let mut output = json!({
        "kind": kind,
        "call_type": logging.call_type,
        "ocr_calls": calls,
        "warned": warnings,
    });
    if let (Some((name, value)), Some(fields)) = (detail, output.as_object_mut()) {
        fields.insert(String::from(name), value);
    }
    output
}

fn collected_step(step: &CollectedStep) -> Value {
    let plan = plan_for(None, None, Object::new(), None);
    let streaming = plan.is_streaming_request(&step.endpoint, &step.request_data);
    let mut logging = RecordingLogging::default();
    let outcome = plan.handle_collected_chunks(
        &step.chunks,
        &mut logging,
        &target(&step.model, &step.endpoint),
    );
    let (kind, _detail) = outcome_fields(&outcome);
    json!({"kind": kind, "call_type": logging.call_type, "streaming": streaming})
}
