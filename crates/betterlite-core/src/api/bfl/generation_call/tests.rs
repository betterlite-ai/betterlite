#![cfg(test)]

mod support;

use std::sync::atomic::Ordering;
use std::time::Duration;

use betterlite_config::types::LiteLlmParams;
use betterlite_models::decisions::Json;
use betterlite_openai::image_generation::reply::ImageDatum;
use indexmap::IndexMap;
use secrecy::{ExposeSecret, SecretString};

use super::error::GenerationCallError;
use super::params::{CallParams, ParamsError};
use super::submit::{SuppliedTransport, TransportKind};
use super::{GenerationCall, GenerationExecutor, LogContext};
use crate::api::bfl::generation::BflImageResponse;
use crate::api::bfl::poll::PollError;
use support::{
    BASE, MODEL, PROMPT, READY, RecordedPost, Recorder, SUBMITTED, Script, Sink, StepClock,
    logging_env, logging_state, poll_once, reply,
};

fn key() -> CallParams {
    CallParams {
        api_key: Some(SecretString::from(String::from("sk-bfl"))),
        api_base: Some(String::from(BASE)),
    }
}

fn generation_call<'text>(
    optional_params: &'text IndexMap<String, Json>,
    params: CallParams,
) -> GenerationCall<'text> {
    GenerationCall {
        model: MODEL,
        prompt: PROMPT,
        optional_params,
        params,
        extra_headers: None,
        timeout: None,
        aimg_generation: false,
        configured_key: None,
        configured_base: BASE,
        now_seconds: 1_700_000_000,
    }
}

struct Outcome {
    result: Result<BflImageResponse, GenerationCallError>,
    details: IndexMap<String, Json>,
}

async fn run(
    script: &Script,
    clock: &StepClock,
    sleeper: &Recorder<'_>,
    call: &GenerationCall<'_>,
    data: Vec<ImageDatum>,
) -> Outcome {
    let (redactor, mut state) = logging_state();
    let sink = Sink::default();
    let env = logging_env(&redactor, &sink);
    let executor = GenerationExecutor {
        provider: script,
        clock,
        sleeper,
    };
    let supplied: Option<SuppliedTransport<'_, Script>> = None;
    let result = {
        let mut log = LogContext {
            state: &mut state,
            env: &env,
        };
        executor
            .image_generation(call, supplied, data, &mut log)
            .await
    };
    Outcome {
        result,
        details: state.model_call_details.clone(),
    }
}

fn object(value: Option<&Json>) -> &IndexMap<String, Json> {
    match value {
        Some(Json::Object(fields)) => fields,
        _ => panic!("the fixture field is an object"),
    }
}

fn text(value: Option<&Json>) -> &str {
    match value {
        Some(Json::Text(found)) => found,
        _ => panic!("the fixture field is text"),
    }
}

fn value_of(post: &RecordedPost, name: &str) -> String {
    post.headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .expect("the submission carries the header")
        .to_owned()
}

#[tokio::test]
async fn the_pre_call_records_the_prompt_url_headers_and_body() {
    let script = Script::ready();
    let clock = StepClock::stepping(0);
    let sleeper = Recorder::new(&clock);
    let optional_params = IndexMap::new();
    let call = generation_call(&optional_params, key());

    let outcome = run(&script, &clock, &sleeper, &call, Vec::new()).await;

    assert!(outcome.result.is_ok());
    assert_eq!(text(outcome.details.get("input")), PROMPT);
    assert_eq!(text(outcome.details.get("log_event_type")), "pre_api_call");
    let additional = object(outcome.details.get("additional_args"));
    assert_eq!(
        text(additional.get("api_base")),
        format!("{BASE}/v1/flux-pro-1.1")
    );
    let headers = object(additional.get("headers"));
    assert!(headers.contains_key("x-key"));
    assert_eq!(text(headers.get("content-type")), "application/json");
    assert_eq!(text(headers.get("accept")), "application/json");
    let body = object(additional.get("complete_input_dict"));
    assert_eq!(text(body.get("prompt")), PROMPT);
    assert_eq!(text(body.get("output_format")), "png");
}

#[tokio::test]
async fn the_submission_carries_the_validated_headers_and_the_requested_timeout() {
    let script = Script::ready();
    let clock = StepClock::stepping(0);
    let sleeper = Recorder::new(&clock);
    let optional_params = IndexMap::new();
    let mut call = generation_call(&optional_params, key());
    call.timeout = Some(Duration::from_millis(12_500));

    let outcome = run(&script, &clock, &sleeper, &call, Vec::new()).await;

    assert!(outcome.result.is_ok());
    let post = script.last_post();
    assert_eq!(post.url, format!("{BASE}/v1/flux-pro-1.1"));
    assert_eq!(post.timeout, Some(Duration::from_millis(12_500)));
    assert_eq!(value_of(&post, "x-key"), "sk-bfl");
    assert_eq!(value_of(&post, "content-type"), "application/json");
    assert_eq!(value_of(&post, "accept"), "application/json");
    let expected = IndexMap::from([
        (String::from("prompt"), Json::Text(String::from(PROMPT))),
        (
            String::from("output_format"),
            Json::Text(String::from("png")),
        ),
    ]);
    assert_eq!(post.body, Json::Object(expected));
}

#[tokio::test]
async fn an_absent_timeout_leaves_the_submission_deadline_to_the_client() {
    let script = Script::ready();
    let clock = StepClock::stepping(0);
    let sleeper = Recorder::new(&clock);
    let optional_params = IndexMap::new();
    let call = generation_call(&optional_params, key());

    let outcome = run(&script, &clock, &sleeper, &call, Vec::new()).await;

    assert!(outcome.result.is_ok());
    assert_eq!(script.last_post().timeout, None);
}

#[tokio::test]
async fn extra_headers_replace_the_validated_headers_and_the_polling_key() {
    let script = Script::ready();
    let clock = StepClock::stepping(0);
    let sleeper = Recorder::new(&clock);
    let optional_params = IndexMap::new();
    let extra = [
        (String::from("x-key"), String::from("caller-key")),
        (String::from("X-Trace"), String::from("t-1")),
    ];
    let mut call = generation_call(&optional_params, key());
    call.extra_headers = Some(&extra);

    let outcome = run(&script, &clock, &sleeper, &call, Vec::new()).await;

    assert!(outcome.result.is_ok());
    let post = script.last_post();
    assert_eq!(value_of(&post, "x-key"), "caller-key");
    assert_eq!(value_of(&post, "x-trace"), "t-1");
    let gets = script.gets.lock().expect("the gets lock is not poisoned");
    assert_eq!(gets.len(), 1);
    assert_eq!(
        gets[0]
            .headers
            .get("x-key")
            .and_then(|value| value.to_str().ok()),
        Some("caller-key")
    );
    assert_eq!(gets[0].headers.len(), 1);
}

#[tokio::test]
async fn a_rejected_submission_is_a_500_and_stops_before_a_poll() {
    let script = Script::new(reply(400, r#"{"detail":"bad prompt"}"#), Vec::new());
    let clock = StepClock::stepping(0);
    let sleeper = Recorder::new(&clock);
    let optional_params = IndexMap::new();
    let call = generation_call(&optional_params, key());

    let outcome = run(&script, &clock, &sleeper, &call, Vec::new()).await;

    let error = outcome.result.expect_err("a rejected submission fails");
    assert!(matches!(
        &error,
        GenerationCallError::SubmissionStatus { status: 400 }
    ));
    assert_eq!(error.status(), Some(500));
    assert!(
        script
            .gets
            .lock()
            .expect("the gets lock is not poisoned")
            .is_empty()
    );
}

#[tokio::test]
async fn a_created_submission_is_polled_like_a_200() {
    let script = Script::new(reply(201, SUBMITTED), vec![reply(200, READY)]);
    let clock = StepClock::stepping(0);
    let sleeper = Recorder::new(&clock);
    let optional_params = IndexMap::new();
    let call = generation_call(&optional_params, key());

    let outcome = run(&script, &clock, &sleeper, &call, Vec::new()).await;

    let response = outcome.result.expect("a 201 submission is accepted");
    assert_eq!(response.data.len(), 1);
    assert_eq!(
        response.data[0].url.as_deref(),
        Some("https://delivery.test/a.png")
    );
    assert_eq!(response.created, 1_700_000_000);
}

#[tokio::test]
async fn a_poll_answering_created_fails_with_its_own_status() {
    let script = Script::new(
        reply(200, SUBMITTED),
        vec![reply(201, r#"{"status":"Pending"}"#)],
    );
    let clock = StepClock::stepping(0);
    let sleeper = Recorder::new(&clock);
    let optional_params = IndexMap::new();
    let call = generation_call(&optional_params, key());

    let outcome = run(&script, &clock, &sleeper, &call, Vec::new()).await;

    let error = outcome.result.expect_err("only a 200 poll is accepted");
    assert_eq!(error.status(), Some(201));
}

#[tokio::test]
async fn the_image_result_is_appended_to_the_supplied_data() {
    let script = Script::ready();
    let clock = StepClock::stepping(0);
    let sleeper = Recorder::new(&clock);
    let optional_params = IndexMap::new();
    let call = generation_call(&optional_params, key());
    let existing = ImageDatum {
        b64_json: None,
        url: Some(String::from("https://delivery.test/old.png")),
        revised_prompt: None,
        provider_specific_fields: None,
    };

    let outcome = run(&script, &clock, &sleeper, &call, vec![existing]).await;

    let response = outcome.result.expect("a ready job returns images");
    let urls: Vec<Option<&str>> = response
        .data
        .iter()
        .map(|datum| datum.url.as_deref())
        .collect();
    assert_eq!(
        urls,
        vec![
            Some("https://delivery.test/old.png"),
            Some("https://delivery.test/a.png")
        ]
    );
}

#[tokio::test]
async fn a_ready_reply_without_an_image_is_a_500() {
    let script = Script::new(
        reply(200, SUBMITTED),
        vec![reply(200, r#"{"status":"Ready","result":{}}"#)],
    );
    let clock = StepClock::stepping(0);
    let sleeper = Recorder::new(&clock);
    let optional_params = IndexMap::new();
    let call = generation_call(&optional_params, key());

    let outcome = run(&script, &clock, &sleeper, &call, Vec::new()).await;

    let error = outcome.result.expect_err("a result without a URL fails");
    assert_eq!(error.status(), Some(500));
}

#[tokio::test]
async fn a_moderated_poll_is_a_400_with_the_literal_message() {
    let script = Script::new(
        reply(200, SUBMITTED),
        vec![reply(200, r#"{"status":"Content Moderated"}"#)],
    );
    let clock = StepClock::stepping(0);
    let sleeper = Recorder::new(&clock);
    let optional_params = IndexMap::new();
    let call = generation_call(&optional_params, key());

    let outcome = run(&script, &clock, &sleeper, &call, Vec::new()).await;

    let error = outcome.result.expect_err("a moderated job fails");
    assert!(matches!(
        &error,
        GenerationCallError::Poll(PollError::Failed { .. })
    ));
    assert_eq!(error.status(), Some(400));
    assert_eq!(
        error.to_string(),
        "Image generation failed: Content Moderated"
    );
}

#[tokio::test]
async fn missing_credentials_fail_before_a_submission() {
    let script = Script::ready();
    let clock = StepClock::stepping(0);
    let sleeper = Recorder::new(&clock);
    let optional_params = IndexMap::new();
    let call = generation_call(
        &optional_params,
        CallParams {
            api_key: None,
            api_base: None,
        },
    );

    let outcome = run(&script, &clock, &sleeper, &call, Vec::new()).await;

    let error = outcome.result.expect_err("an absent key fails");
    assert_eq!(error.status(), Some(401));
    assert!(
        script
            .posts
            .lock()
            .expect("the posts lock is not poisoned")
            .is_empty()
    );
}

#[test]
fn a_dict_params_reads_the_key_and_base() {
    let mut dict = IndexMap::new();
    dict.insert(String::from("api_key"), Json::Text(String::from("sk-dict")));
    dict.insert(String::from("api_base"), Json::Text(String::from(BASE)));

    let params = CallParams::from_dict(&dict).expect("the dict params are typed");

    assert_eq!(
        params.api_key.as_ref().map(|key| key.expose_secret()),
        Some("sk-dict")
    );
    assert_eq!(params.api_base.as_deref(), Some(BASE));
}

#[test]
fn a_dict_params_naming_its_fields_rejects_a_non_text_value() {
    let mut dict = IndexMap::new();
    dict.insert(String::from("api_key"), Json::Number(7.into()));

    let error = CallParams::from_dict(&dict).expect_err("a number is not a key");

    assert!(matches!(error, ParamsError::FieldType { field: "api_key" }));
    assert_eq!(error.to_string(), "litellm_params.api_key must be a string");
}

#[test]
fn typed_params_read_the_key_and_base() {
    let params = CallParams::from_typed(&LiteLlmParams {
        ssl_verify: None,
        model: None,
        api_base: Some(String::from(BASE)),
        api_key: Some(SecretString::from(String::from("sk-typed"))),
        api_version: None,
        timeout: None,
        stream_timeout: None,
        max_retries: None,
        rpm: None,
        tpm: None,
        weight: None,
    });

    assert_eq!(
        params.api_key.as_ref().map(|key| key.expose_secret()),
        Some("sk-typed")
    );
    assert_eq!(params.api_base.as_deref(), Some(BASE));
}

#[tokio::test]
async fn a_sync_call_uses_a_sync_supplied_transport() {
    let provider = Script::new(reply(500, ""), Vec::new());
    let supplied = Script::ready();
    let clock = StepClock::stepping(0);
    let sleeper = Recorder::new(&clock);
    let optional_params = IndexMap::new();
    let call = generation_call(&optional_params, key());
    let executor = GenerationExecutor {
        provider: &provider,
        clock: &clock,
        sleeper: &sleeper,
    };
    let handed = SuppliedTransport {
        kind: TransportKind::Sync,
        transport: &supplied,
    };
    let (redactor, mut state) = logging_state();
    let sink = Sink::default();
    let env = logging_env(&redactor, &sink);
    let mut log = LogContext {
        state: &mut state,
        env: &env,
    };

    let result = executor
        .image_generation(&call, Some(handed), Vec::new(), &mut log)
        .await;

    assert!(result.is_ok());
    assert_eq!(
        supplied
            .posts
            .lock()
            .expect("the posts lock is not poisoned")
            .len(),
        1
    );
    assert!(
        provider
            .posts
            .lock()
            .expect("the posts lock is not poisoned")
            .is_empty()
    );
}

#[tokio::test]
async fn a_sync_call_discards_an_async_supplied_transport() {
    let provider = Script::ready();
    let supplied = Script::new(reply(500, ""), Vec::new());
    let clock = StepClock::stepping(0);
    let sleeper = Recorder::new(&clock);
    let optional_params = IndexMap::new();
    let call = generation_call(&optional_params, key());
    let executor = GenerationExecutor {
        provider: &provider,
        clock: &clock,
        sleeper: &sleeper,
    };
    let handed = SuppliedTransport {
        kind: TransportKind::Async,
        transport: &supplied,
    };
    let (redactor, mut state) = logging_state();
    let sink = Sink::default();
    let env = logging_env(&redactor, &sink);
    let mut log = LogContext {
        state: &mut state,
        env: &env,
    };

    let result = executor
        .image_generation(&call, Some(handed), Vec::new(), &mut log)
        .await;

    assert!(result.is_ok());
    assert_eq!(
        provider
            .posts
            .lock()
            .expect("the posts lock is not poisoned")
            .len(),
        1
    );
    assert!(
        supplied
            .posts
            .lock()
            .expect("the posts lock is not poisoned")
            .is_empty()
    );
}

#[tokio::test]
async fn an_async_call_discards_a_sync_supplied_transport() {
    let provider = Script::ready();
    let supplied = Script::new(reply(500, ""), Vec::new());
    let clock = StepClock::stepping(0);
    let sleeper = Recorder::new(&clock);
    let optional_params = IndexMap::new();
    let mut call = generation_call(&optional_params, key());
    call.aimg_generation = true;
    let executor = GenerationExecutor {
        provider: &provider,
        clock: &clock,
        sleeper: &sleeper,
    };
    let handed = SuppliedTransport {
        kind: TransportKind::Sync,
        transport: &supplied,
    };
    let (redactor, mut state) = logging_state();
    let sink = Sink::default();
    let env = logging_env(&redactor, &sink);
    let mut log = LogContext {
        state: &mut state,
        env: &env,
    };

    let result = executor
        .image_generation(&call, Some(handed), Vec::new(), &mut log)
        .await;

    assert!(result.is_ok());
    assert_eq!(
        provider
            .posts
            .lock()
            .expect("the posts lock is not poisoned")
            .len(),
        1
    );
    assert!(
        supplied
            .posts
            .lock()
            .expect("the posts lock is not poisoned")
            .is_empty()
    );
}

#[test]
fn dropping_a_pending_generation_call_cancels_the_submission() {
    let mut script = Script::ready();
    script.hang_post = true;
    let clock = StepClock::stepping(0);
    let sleeper = Recorder::new(&clock);
    let optional_params = IndexMap::new();
    let call = generation_call(&optional_params, key());
    let (redactor, mut state) = logging_state();
    let sink = Sink::default();
    let env = logging_env(&redactor, &sink);
    let executor = GenerationExecutor {
        provider: &script,
        clock: &clock,
        sleeper: &sleeper,
    };
    let mut log = LogContext {
        state: &mut state,
        env: &env,
    };
    let supplied: Option<SuppliedTransport<'_, Script>> = None;

    let mut future = Box::pin(executor.image_generation(&call, supplied, Vec::new(), &mut log));
    assert!(poll_once(future.as_mut()).is_pending());
    drop(future);

    assert!(script.cancelled.load(Ordering::SeqCst));
}

#[test]
fn dropping_a_pending_generation_call_cancels_the_poll() {
    let mut script = Script::ready();
    script.hang_get = true;
    let clock = StepClock::stepping(0);
    let sleeper = Recorder::new(&clock);
    let optional_params = IndexMap::new();
    let call = generation_call(&optional_params, key());
    let (redactor, mut state) = logging_state();
    let sink = Sink::default();
    let env = logging_env(&redactor, &sink);
    let executor = GenerationExecutor {
        provider: &script,
        clock: &clock,
        sleeper: &sleeper,
    };
    let mut log = LogContext {
        state: &mut state,
        env: &env,
    };
    let supplied: Option<SuppliedTransport<'_, Script>> = None;

    let mut future = Box::pin(executor.image_generation(&call, supplied, Vec::new(), &mut log));
    assert!(poll_once(future.as_mut()).is_pending());
    drop(future);

    assert!(script.cancelled.load(Ordering::SeqCst));
}
