use std::time::Duration;

use betterlite_openai::speech::request::SpeechOptions;
use http::Method;
use rust_decimal::Decimal;
use serde_json::Value;

use super::super::call::SpeechCall;
use super::support::*;

fn native_call() -> SpeechCall {
    SpeechCall {
        voice: named_voice("alloy"),
        response_format: Some(String::from("wav")),
        speed: Some(Decimal::ZERO),
        instructions: Some(String::new()),
        headers: Some(object(r#"{"x": "ignored"}"#)),
        kwargs: object(
            r#"{"tags": ["a"], "user": "u-1", "extra_headers": {"x-extra": "ignored"}}"#,
        ),
        ..base_call("openai/tts-1")
    }
}

#[test]
fn native_defaults_post_the_audio_speech_body() {
    let variables = [("OPENAI_API_KEY", "sk-default")];
    let harness = Harness::new(
        loaded_settings(SOURCE, &variables),
        RecordingTransport::ok("audio/mpeg", &[0, 1, 2]),
        &variables,
    );
    let content = harness.run_ok(&native_call());
    assert_eq!(content.status, 200);
    assert_eq!(content.body, [0, 1, 2]);
    assert_eq!(
        content.headers.get("content-type").map(String::as_str),
        Some("audio/mpeg")
    );
    assert!(harness.completions.calls().is_empty());
    assert!(harness.operations.take().is_empty());
    let sent = harness.transport.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, Method::POST);
    assert_eq!(sent[0].url.path(), "/v1/audio/speech");
    assert_eq!(sent[0].deadline, Duration::from_secs(6_000));
    assert_eq!(
        header(&sent[0], "authorization").as_deref(),
        Some("Bearer sk-default")
    );
    assert_eq!(
        header(&sent[0], "accept").as_deref(),
        Some("application/octet-stream")
    );
    assert_eq!(header(&sent[0], "x"), None);
    assert_eq!(header(&sent[0], "x-extra"), None);
    assert_eq!(
        sent_json(&sent[0]),
        serde_json::json!({
            "input": "hello",
            "model": "tts-1",
            "voice": "alloy",
            "response_format": "wav",
            "speed": 0,
            "instructions": "",
        })
    );
}

// LiteLLM builds `headers or litellm.headers` and never passes it to audio_speech
#[test]
fn configured_global_headers_stay_out_of_the_request() {
    let variables = [("OPENAI_API_KEY", "sk-global-headers")];
    let harness = Harness::new(
        loaded_settings(
            "litellm_settings: {headers: {x-global: 'no'}}\n",
            &variables,
        ),
        RecordingTransport::ok("audio/mpeg", &[1]),
        &variables,
    );
    let native = SpeechCall {
        voice: named_voice("alloy"),
        ..base_call("openai/tts-1")
    };
    harness.run_ok(&native);
    let sent = harness.transport.sent();
    assert_eq!(header(&sent[0], "x-global"), None);
}

#[test]
fn native_defaults_use_the_speech_timeout_and_retries() {
    let settings = loaded_settings(SOURCE, &[]);
    let native = base_call("openai/tts-1");
    assert_eq!(
        native.timeout_seconds(&settings).to_bits(),
        6000.0_f64.to_bits()
    );
    assert_eq!(native.max_retries(&settings), 2_u32);
}

#[test]
fn explicit_zero_timeout_and_retries_stay() {
    let settings = loaded_settings(SOURCE, &[]);
    let native = SpeechCall {
        timeout_seconds: Some(0.0_f64),
        max_retries: Some(0_u32),
        ..base_call("openai/tts-1")
    };
    assert_eq!(
        native.timeout_seconds(&settings).to_bits(),
        0.0_f64.to_bits()
    );
    assert_eq!(native.max_retries(&settings), 0_u32);
}

#[test]
fn configured_timeout_and_retries_apply() {
    let settings = loaded_settings(
        "litellm_settings: {request_timeout: 30.5, num_retries: 4}\n",
        &[],
    );
    let native = base_call("openai/tts-1");
    assert_eq!(
        native.timeout_seconds(&settings).to_bits(),
        30.5_f64.to_bits()
    );
    assert_eq!(native.max_retries(&settings), 4_u32);
}

#[test]
fn the_request_timeout_environment_fills_the_default() {
    let settings = loaded_settings(SOURCE, &[("REQUEST_TIMEOUT", "12.5")]);
    assert_eq!(
        settings.request_timeout_seconds.to_bits(),
        12.5_f64.to_bits()
    );
    assert_eq!(
        base_call("openai/tts-1")
            .timeout_seconds(&settings)
            .to_bits(),
        12.5_f64.to_bits()
    );
}

#[test]
fn global_zero_retries_fall_back_to_two() {
    let settings = loaded_settings("litellm_settings: {num_retries: 0}\n", &[]);
    assert_eq!(base_call("openai/tts-1").max_retries(&settings), 2_u32);
}

#[test]
fn initial_options_keep_explicit_zero_and_empty() {
    let params = native_call()
        .initial_optional_params()
        .expect("the options build");
    assert_eq!(
        Value::Object(params),
        serde_json::json!({"response_format": "wav", "speed": 0, "instructions": ""})
    );
}

#[test]
fn unset_options_stay_out() {
    let params = base_call("openai/tts-1")
        .initial_optional_params()
        .expect("the options build");
    assert!(params.is_empty());
}

#[test]
fn native_options_deserialize_into_the_sdk_options() {
    let params = native_call()
        .initial_optional_params()
        .expect("the options build");
    let sdk_options: SpeechOptions =
        serde_json::from_value(Value::Object(params)).expect("the SDK options");
    assert_eq!(sdk_options.speed, Some(Decimal::ZERO));
    assert_eq!(sdk_options.instructions.as_deref(), Some(""));
    assert_eq!(
        sdk_options.response_format.map(|format| format.wire_name()),
        Some("wav")
    );
}
