use std::time::Duration;

use betterlite_azure::endpoints::projection::{LiteLlmParams, SessionKind};
use betterlite_azure::speech::binary::{AudioSpeechCall, finish_audio_speech};
use betterlite_azure::speech::dispatch::{
    DispatchCall, SpeechEnvironment, SpeechGlobals, dispatch_text_to_speech,
    supported_openai_params,
};
use betterlite_azure::speech::error::AzureSpeechError;
use betterlite_azure::speech::probe::{ProbeCall, ProbeMode, finish_probe, probe_request};
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use super::fixtures::handler::{HandlerRecord, Recorder, RecordingHandler};
use super::fixtures::reply::upstream;
use super::fixtures::wiring::{Inputs, start};
use super::input::{AudioSpeech, Input, NativeCall, Outcome, ProbeModeInput, ProbeScenario};

pub(crate) fn run(input: &Input) -> HelperResult {
    match input {
        Input::AudioSpeech(case) => audio_view(case),
        Input::SupportedParams(case) => supported_view(case),
        Input::NativeSpeechSelection(case) => {
            let driver = blocking_runtime()?;
            let mut views = Vec::with_capacity(case.calls.len());
            for call in &case.calls {
                views.push(native_view(&driver, call)?);
            }
            Ok(Value::Array(views))
        }
        Input::Probe(case) => {
            let mut views = Vec::with_capacity(case.scenarios.len());
            for scenario in &case.scenarios {
                views.push(probe_view(scenario)?);
            }
            Ok(Value::Array(views))
        }
    }
}

// AudioSpeechConfig.get_supported_openai_params beside the one speech the same case sends.
fn supported_view(case: &AudioSpeech) -> HelperResult {
    Ok(json!({
        "supported": supported_openai_params(),
        "speech": audio_view(case)?,
    }))
}

// audio_speech's wire request, its pre_call record, and either the binary reply or the failure it raises.
fn audio_view(case: &AudioSpeech) -> HelperResult {
    let call = audio_call(case);
    let mut inputs = Inputs::default();
    let mut logger = Recorder::default();
    let request = match start(&mut inputs, &call, &mut logger) {
        Ok(request) => request,
        Err(error) => {
            let pre_call = pre_call_view(&logger);
            return Ok(audio_answer(Value::Null, pre_call, Some(&error)));
        }
    };
    let sent = upstream(&case.reply)?;
    let pre_call = pre_call_view(&logger);
    let view = request_view(&request)?;
    match finish_audio_speech(sent) {
        Ok(content) => Ok(json!({
            "request": view,
            "pre_call": pre_call,
            "result": {
                "status": content.status,
                "content": String::from_utf8_lossy(&content.body),
            },
            "error": Value::Null,
        })),
        Err(error) => Ok(audio_answer(view, pre_call, Some(&error))),
    }
}

fn audio_answer(request: Value, pre_call: Value, error: Option<&AzureSpeechError>) -> Value {
    json!({
        "request": request,
        "pre_call": pre_call,
        "result": Value::Null,
        "error": error.map_or(Value::Null, error_view),
    })
}

fn audio_call(case: &AudioSpeech) -> AudioSpeechCall<'_> {
    AudioSpeechCall {
        model: &case.model,
        input: &case.input,
        voice: &case.voice,
        optional_params: case.optional_params.clone(),
        aspeech: case.aspeech,
        api_key: optional_secret(case.api_key.as_deref()),
        api_base: Some(case.api_base.clone()),
        api_version: Some(case.api_version.clone()),
        params: LiteLlmParams::default(),
        provider: None,
        supplied: None,
        session: SessionKind::Dedicated,
        refresh_enabled: false,
    }
}

fn pre_call_view(logger: &Recorder) -> Value {
    match &logger.pre {
        Some(record) => json!({
            "input": record.input,
            "api_base": record.api_base,
            "data": record.data,
        }),
        None => Value::Null,
    }
}

// The token the dispatch hands the text-to-speech handler: what it resolved and how it resolved it.
fn native_view(runtime: &Runtime, call: &NativeCall) -> HelperResult {
    let globals = SpeechGlobals {
        api_base: call.globals.api_base.clone(),
        api_key: optional_secret(call.globals.api_key.as_deref()),
        azure_key: optional_secret(call.globals.azure_key.as_deref()),
    };
    let environment = SpeechEnvironment {
        globals: &globals,
        environment: &call.environment,
    };
    let dispatch = DispatchCall {
        model: &call.model,
        input: &call.input,
        voice: call.voice.as_ref(),
        optional_params: &call.optional_params,
        litellm_params: call.litellm_params.clone(),
        api_base: call.api_base.clone(),
        api_key: optional_secret(&call.api_key),
        timeout: Some(Duration::from_secs(call.timeout_seconds)),
        extra_headers: call.extra_headers.as_ref(),
        aspeech: call.aspeech,
        environment: &environment,
    };
    let mut handler = RecordingHandler::default();
    let outcome = runtime.block_on(dispatch_text_to_speech(dispatch, &mut handler));
    let Some(record) = handler.requests.into_iter().next() else {
        return Err(fixture_failure("the dispatch reaches the handler"));
    };
    let (answer, error) = match outcome {
        Ok(answer) => (Value::String(answer), Value::Null),
        Err(failure) => (Value::Null, error_view(&failure)),
    };
    Ok(record_view(record, answer, error))
}

fn record_view(record: HandlerRecord, answer: Value, error: Value) -> Value {
    json!({
        "voice": record.voice,
        "litellm_params": record.litellm_params,
        "optional_params": record.optional_params,
        "extra_headers": record.extra_headers,
        "timeout": record.timeout.map(|timeout| timeout.as_secs().to_string()),
        "provider": record.provider,
        "is_async": record.is_async,
        "client": Value::Null,
        "result": answer,
        "error": error,
    })
}

fn probe_view(scenario: &ProbeScenario) -> HelperResult {
    if matches!(scenario.outcome, Outcome::InvalidCompletion) {
        let error = AzureSpeechError::InvalidCompletion;
        return Ok(probe_answer(Value::Null, Value::Null, Some(&error)));
    }
    let request = match probe_request(&probe_call(scenario)) {
        Ok(request) => request,
        Err(error) => return Ok(probe_answer(Value::Null, Value::Null, Some(&error))),
    };
    let view = request_view(&request)?;
    if matches!(scenario.outcome, Outcome::Request) {
        return Ok(probe_answer(view, Value::Null, None));
    }
    let Some(reply) = &scenario.reply else {
        return Err(fixture_failure("a reply scenario carries its reply"));
    };
    match finish_probe(Some(upstream(reply)?)) {
        Ok(headers) => Ok(probe_answer(view, json!({"headers": headers}), None)),
        Err(error) => Ok(probe_answer(view, Value::Null, Some(&error))),
    }
}

fn probe_answer(request: Value, reply: Value, error: Option<&AzureSpeechError>) -> Value {
    json!({
        "request": request,
        "reply": reply,
        "error": error.map_or(Value::Null, error_view),
    })
}

fn probe_call(scenario: &ProbeScenario) -> ProbeCall<'_> {
    ProbeCall {
        model: scenario.model.as_deref(),
        api_key: &scenario.api_key,
        api_base: scenario.api_base.as_deref(),
        api_version: scenario.api_version.as_deref(),
        timeout: None,
        mode: match scenario.mode {
            ProbeModeInput::Chat => ProbeMode::Chat,
            ProbeModeInput::ImageGeneration => ProbeMode::ImageGeneration,
        },
        messages: scenario.messages.as_ref(),
    }
}

fn request_view(request: &OutboundRequest) -> HelperResult {
    Ok(json!({
        "method": request.method.as_str(),
        "url": request.url.as_str(),
        "api_key": header(request, "api-key"),
        "authorization": header(request, "authorization"),
        "accept": header(request, "accept"),
        "x_extra": header(request, "x-extra"),
        "body": request_body(&request.body)?,
    }))
}

fn request_body(body: &RequestBody) -> Result<Value, Box<HelperError>> {
    match body {
        RequestBody::Json(bytes) => {
            serde_json::from_slice(bytes).map_err(|error| fixture_failure(&error.to_string()))
        }
        RequestBody::Bytes { data, .. } => {
            Ok(Value::String(String::from_utf8_lossy(data).into_owned()))
        }
        RequestBody::Empty => Ok(Value::Null),
        RequestBody::Multipart { .. } => Err(fixture_failure("a speech request is not multipart")),
    }
}

fn header<'request>(request: &'request OutboundRequest, name: &str) -> Option<&'request str> {
    match request.headers.get(name) {
        Some(value) => value.to_str().ok(),
        None => None,
    }
}

// A status failure is compared as the SDK's status code; any other failure as its class and message.
fn error_view(error: &AzureSpeechError) -> Value {
    match error.status() {
        Some(status) => json!({"status": status}),
        None => json!({"class": error.class(), "message": error.to_string()}),
    }
}

fn optional_secret(value: Option<&str>) -> Option<SecretString> {
    value.map(SecretString::from)
}

fn blocking_runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread().build().map_err(|error| {
        Box::new(HelperError {
            class: String::from("RuntimeError"),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}

fn fixture_failure(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("Fixture"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
