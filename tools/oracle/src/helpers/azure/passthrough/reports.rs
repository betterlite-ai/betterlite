use betterlite_azure::passthrough::config::{
    AzurePassthroughConfig, AzurePassthroughEnvironment, validate_environment,
};
use betterlite_azure::passthrough::endpoint::{
    body_model_inference_endpoint, foreign_azure_deployment, router_model_in_endpoint,
};
use betterlite_azure::passthrough::logging::AzureRelayedLog;
use betterlite_azure::passthrough::url::without_api_version;
use betterlite_openai::api::responses_event_records::records::ResponsesTerminalEvent;
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughUrlRequest, RelayedResponse,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use betterlite_oracle_helper_api::helper::HelperResult;

use super::fixtures::{
    NoSources, OracleChatProjection, RecordingLogging, RecordingStream, base_config,
    config_with_reply, config_with_stream, deployment, target,
};
use super::{
    DeploymentStep, InferenceStep, Input, KeyStep, NonstreamStep, RelayStep, RouterStep,
    StreamStep, UrlStep, ValidationStep,
};

pub(super) fn run(input: &Input) -> HelperResult {
    Ok(match input {
        Input::BodyModelInferenceEndpoint { inference_steps } => inference_report(inference_steps),
        Input::RouterModelInPath { router_steps } => router_report(router_steps),
        Input::ForeignDeployment { deployment_steps } => json!({
            "steps": deployment_steps.iter().map(deployment_step).collect::<Vec<Value>>()
        }),
        Input::ApiVersionRemoval {
            removals,
            url_steps,
        } => removal_report(removals, url_steps),
        Input::ResponsesStreamLog { stream_steps } => json!({
            "steps": stream_steps.iter().map(stream_step).collect::<Vec<Value>>()
        }),
        Input::MessagesRelayLog { relay_steps } => json!({
            "steps": relay_steps.iter().map(relay_step).collect::<Vec<Value>>()
        }),
        Input::NonstreamUsage { nonstream_steps } => json!({
            "steps": nonstream_steps.iter().map(nonstream_step).collect::<Vec<Value>>()
        }),
        Input::PassthroughAuthInputs {
            key_steps,
            validation_steps,
        } => auth_report(key_steps, validation_steps),
    })
}

fn inference_report(steps: &[InferenceStep]) -> Value {
    let steps: Vec<Value> = steps
        .iter()
        .map(|step| {
            let config = base_config(None);
            json!({
                "inference": body_model_inference_endpoint(&step.endpoint),
                "streaming": config.is_streaming_request(&step.endpoint, &step.request_data),
            })
        })
        .collect();
    json!({ "steps": steps })
}

fn router_report(steps: &[RouterStep]) -> Value {
    let steps: Vec<Value> = steps
        .iter()
        .map(|step| json!({ "model": router_model_in_endpoint(&step.endpoint, &step.router_models) }))
        .collect();
    json!({ "steps": steps })
}

fn deployment_step(step: &DeploymentStep) -> Value {
    let deployment = foreign_azure_deployment(&step.endpoint, &step.model_group, || {
        step.served_models.clone()
    });
    json!({ "deployment": deployment })
}

fn removal_report(removals: &[String], steps: &[UrlStep]) -> Value {
    let stripped: Vec<Value> = removals
        .iter()
        .map(|base| match without_api_version(base) {
            Ok(stripped) => Value::String(stripped),
            Err(_error) => Value::Null,
        })
        .collect();
    let steps: Vec<Value> = steps.iter().map(url_step).collect();
    json!({ "removals": stripped, "steps": steps })
}

fn url_step(step: &UrlStep) -> Value {
    let config = base_config(step.env_base.as_deref());
    let request = PassthroughUrlRequest {
        api_base: step.api_base.as_deref(),
        api_key: None,
        target: target(&step.model, &step.endpoint),
        request_query_params: step.request_query_params.as_ref(),
        litellm_params: &step.litellm_params,
    };
    match config.get_complete_url(&request) {
        Ok((url, base)) => json!({ "url": url.as_str(), "base": base }),
        Err(_error) => json!({ "url": Value::Null, "base": Value::Null }),
    }
}

fn stream_step(step: &StreamStep) -> Value {
    let config = base_config(None);
    let mut logging_obj = RecordingLogging::default();
    let logged = config.handle_collected_chunks(
        &step.chunks,
        &mut logging_obj,
        &target("gpt-4o", &step.endpoint),
    );
    let event = match &logged {
        Some(AzureRelayedLog::Terminal(event)) => terminal_view(event),
        Some(AzureRelayedLog::Chat(_) | AzureRelayedLog::Relay(_)) | None => Value::Null,
    };
    json!({ "event": event, "call_type": call_type(&logging_obj) })
}

fn terminal_view(event: &ResponsesTerminalEvent) -> Value {
    let (class, inner) = match event {
        ResponsesTerminalEvent::Completed(inner) => ("ResponseCompletedEvent", inner),
        ResponsesTerminalEvent::Incomplete(inner) => ("ResponseIncompleteEvent", inner),
        ResponsesTerminalEvent::Failed(inner) => ("ResponseFailedEvent", inner),
    };
    json!({
        "class": class,
        "id": inner.response.id.clone(),
        "created_at": inner.response.created_at,
    })
}

fn relay_step(step: &RelayStep) -> Value {
    let mut config = config_with_stream(RecordingStream::default());
    config.model_call_details = step.model_call_details.clone();
    let mut logging_obj = RecordingLogging::default();
    let logged = config.handle_collected_chunks(
        &step.chunks,
        &mut logging_obj,
        &target(&step.model, &step.endpoint),
    );
    let assembly = config.chat_stream.assembly();
    let called = assembly.is_some();
    let messages = assembly.as_ref().map(|assembly| assembly.messages.clone());
    let model = assembly.map_or_else(|| step.model.clone(), |assembly| assembly.model);
    json!({
        "called": called,
        "result": logged.is_some(),
        "model": model,
        "messages": messages,
        "call_type": call_type(&logging_obj),
    })
}

fn nonstream_step(step: &NonstreamStep) -> Value {
    let config = config_with_reply(OracleChatProjection::default());
    let mut logging_obj = RecordingLogging::default();
    let body = serde_json::to_vec(&Value::Object(step.body.clone())).unwrap_or_default();
    let reply = RelayedResponse {
        status_code: step.status,
        body: &body,
    };
    let logged = config.logging_non_streaming_response(
        &target(&step.model, &step.endpoint),
        &reply,
        &step.headers,
        &step.request_data,
        &mut logging_obj,
    );
    let input = config.chat_reply.logged();
    json!({
        "result_some": logged.is_some(),
        "call_type": call_type(&logging_obj),
        "logging_input": input.as_ref().map(|(messages, _body)| messages.clone()),
        "complete_input_dict": input.map(|(_messages, body)| body),
    })
}

fn auth_report(key_steps: &[KeyStep], validation_steps: &[ValidationStep]) -> Value {
    let api_keys: Vec<Value> = key_steps.iter().map(key_step).collect();
    let headers: Vec<Value> = validation_steps.iter().map(validation_step).collect();
    json!({ "api_keys": api_keys, "headers": headers })
}

fn key_step(step: &KeyStep) -> Value {
    let env_key = step
        .env_key
        .as_deref()
        .map(str::to_owned)
        .map(SecretString::from);
    let config = AzurePassthroughConfig {
        api_key_env: env_key,
        ..base_config(None)
    };
    let api_key = step
        .api_key
        .as_deref()
        .map(str::to_owned)
        .map(SecretString::from);
    match config.get_api_key(api_key.as_ref()) {
        Some(key) => Value::String(key.expose_secret().to_owned()),
        None => Value::Null,
    }
}

fn validation_step(step: &ValidationStep) -> Value {
    let credentials = deployment();
    let sources = NoSources;
    let api_key = step
        .api_key
        .as_deref()
        .map(str::to_owned)
        .map(SecretString::from);
    let params_key = step
        .params_key
        .as_deref()
        .map(str::to_owned)
        .map(SecretString::from);
    let configured_key = step
        .configured_key
        .as_deref()
        .map(str::to_owned)
        .map(SecretString::from);
    let request = AzurePassthroughEnvironment {
        headers: step.headers.clone(),
        api_key: api_key.as_ref(),
        params_key: params_key.as_ref(),
        configured_key: configured_key.as_ref(),
        credentials: &credentials,
        refresh_enabled: false,
        provider: None,
        sources: &sources,
    };
    match validate_environment(request) {
        Ok(headers) => serde_json::to_value(headers).unwrap_or(Value::Null),
        Err(_error) => Value::Null,
    }
}

fn call_type(logging_obj: &RecordingLogging) -> Value {
    match logging_obj.call_type {
        Some(call_type) => Value::String(call_type.as_str().to_owned()),
        None => Value::Null,
    }
}
