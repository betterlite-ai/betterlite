use std::collections::BTreeMap;
use std::fmt::Display;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_core::api::runway::speech::RunwaySpeechConfig;
use betterlite_core::api::runway::speech::SUPPORTED_OPENAI_PARAMS;
use betterlite_core::api::runway::speech::dispatch::{
    DispatchCall, DispatchEnvironment, dispatch_text_to_speech,
};
use betterlite_core::api::runway::speech::error::RunwaySpeechError;
use betterlite_core::api::runway::speech::request::speech_request;
use betterlite_core::api::runway::speech::response::RunwaySpeechTransform;
use betterlite_core::api::speech_call::operations::{
    SpeechConfigOperation as _, SpeechMappingOptions,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::openai_base_config::Object;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use super::fakes::{Clock, Recorder, Script};
use super::inputs::{DispatchInput, Globals, Input, Mapping, RequestInput, ResponseInput};

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "RunwaySpeechError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

// the parse failure keeps the submission status, the way the source's error class does
fn speech_error(error: &RunwaySpeechError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "RunwaySpeechError".to_owned(),
        status: error.submission_status(),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(failure)
}

fn module_globals(globals: &Globals) -> LiteLlmGlobals {
    LiteLlmGlobals {
        api_base: globals.api_base.clone(),
        api_key: globals.api_key.as_deref().map(SecretString::from),
        openai_key: None,
        organization: None,
    }
}

fn header_map(values: &BTreeMap<String, String>) -> Result<HeaderMap, Box<HelperError>> {
    let mut headers = HeaderMap::new();
    for (name, value) in values {
        let header_name = HeaderName::try_from(name.as_str()).map_err(failure)?;
        let header_value = HeaderValue::try_from(value.as_str()).map_err(failure)?;
        headers.insert(header_name, header_value);
    }
    Ok(headers)
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::Mapping(params) => mapping(params),
        Input::Request(params) => Ok(request(params)),
        Input::Dispatch(params) => dispatch(params),
        Input::Response(params) => response(params),
    }
}

fn mapping(input: &Mapping) -> HelperResult {
    let config = RunwaySpeechConfig::for_model(&input.model, &input.environment)
        .map_err(|error| speech_error(&error))?;
    let mut results = Vec::with_capacity(input.calls.len());
    for call in &input.calls {
        let (voice, params) = config.map_openai_params(
            &input.model,
            call.optional_params.clone(),
            &SpeechMappingOptions {
                voice: call.voice.clone(),
                drop_params: call.drop_params,
                kwargs: Object::new(),
            },
        );
        results.push(json!({"voice": voice, "params": params}));
    }
    Ok(json!({"supported": SUPPORTED_OPENAI_PARAMS, "results": results}))
}

fn request(input: &RequestInput) -> Value {
    let data = speech_request(
        &input.model,
        &input.input,
        &input.optional_params,
        &input.headers,
    );
    json!({"dict_body": data.dict_body, "headers": data.headers})
}

fn dispatch(input: &DispatchInput) -> HelperResult {
    let config = RunwaySpeechConfig::for_model(&input.model, &input.environment)
        .map_err(|error| speech_error(&error))?;
    let globals = module_globals(&input.globals);
    let environment = DispatchEnvironment {
        globals: &globals,
        settings: &config.settings,
    };
    let mut recorder = Recorder::default();
    let call = DispatchCall {
        model: &input.model,
        input: &input.input,
        voice: input.voice.as_ref(),
        optional_params: &input.optional_params,
        litellm_params: input.litellm_params.clone(),
        api_base: input.api_base.clone(),
        api_key: input.api_key.as_deref().map(SecretString::from),
        timeout_seconds: input.timeout_seconds,
        extra_headers: input.extra_headers.as_ref(),
        aspeech: input.aspeech,
        environment: &environment,
    };
    runtime()?
        .block_on(dispatch_text_to_speech(call, &mut recorder))
        .map_err(|error| speech_error(&error))?;
    Ok(recorder.calls.pop().unwrap_or(Value::Null))
}

fn response(input: &ResponseInput) -> HelperResult {
    let config = RunwaySpeechConfig::for_model(&input.model, &input.environment)
        .map_err(|error| speech_error(&error))?;
    let transport = Script::new(&input.replies);
    let clock = Clock::new(input.step_millis);
    let transform = RunwaySpeechTransform {
        transport: &transport,
        clock: &clock,
        settings: &config.settings,
    };
    let outgoing = UpstreamResponse {
        status: input.submission.status,
        headers: header_map(&input.submission.headers)?,
        body: input.submission.body.as_bytes().to_vec(),
    };
    let finished = runtime()?
        .block_on(transform.transform(&outgoing, &input.request_headers))
        .map_err(|error| speech_error(&error))?;
    Ok(json!({
        "response": {
            "status": finished.status,
            "content_type": finished.headers.get("content-type"),
            "body_b64": STANDARD.encode(&finished.body),
        },
        "gets": transport.describe(),
        "sleeps": clock.pauses(),
    }))
}
