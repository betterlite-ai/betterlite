use std::future::Future;
use std::task::{Context, Waker};

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::api::audio_file_input::AudioFile;
use http::Method;
use serde_json::json;

use super::support::{
    self, AUDIO_CONTENT, AUDIO_NAME, MODEL, TRANSCRIPTIONS_URL, UPLOAD_URL, answer, call,
    pipeline_with, response, target,
};
use crate::api::soniox::pipeline::error::PipelineError;

fn audio_answers() -> Vec<support::Reply> {
    vec![
        answer(200, r#"{"id":"file_1"}"#),
        answer(200, r#"{"id":"tr_1"}"#),
        answer(200, r#"{"status":"completed","audio_duration_ms":1500}"#),
    ]
}

#[tokio::test]
async fn a_create_failure_deletes_only_the_uploaded_file() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let answers = vec![
        answer(200, r#"{"id":"file_1"}"#),
        answer(500, r#"{"error_message":"bad model"}"#),
        answer(200, "{}"),
    ];
    let recorder = support::Recorder::new(answers, &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = super::support::params(&[("soniox_max_polling_attempts", json!(1))]);
    let call = call(&case_params, false);
    let mut stream = support::NamedStream::new(AUDIO_NAME, AUDIO_CONTENT);
    let mut file = AudioFile::Stream(&mut stream);
    let mut response = response();
    let mut logger = support::Logger::new(&effects, &book);
    let outcome = pipeline
        .transcribe::<support::Recorder<'_>>(
            &call,
            target(Some(&mut file), &mut response, &mut logger),
            None,
        )
        .await;

    let error = outcome.expect_err("the create rejection ends the job");
    let exception = error.provider().expect("the rejection is the Soniox 500");
    assert_eq!(exception.status_code, 500);
    assert_eq!(
        exception.message,
        "Soniox create transcription failed (HTTP 500): bad model"
    );
    assert_eq!(
        effects.kinds(),
        ["request", "pre_call", "request", "request"]
    );
    let calls = recorder.calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[2].method, Method::DELETE);
    assert_eq!(calls[2].url, "https://api.soniox.test/v1/files/file_1");
    assert!(
        calls
            .iter()
            .all(|call| !call.url.contains("/transcriptions/"))
    );
    assert_eq!(response.text.as_deref(), Some(""));
    assert!(response.hidden_params.is_empty());
}

#[tokio::test]
async fn a_fetch_failure_deletes_the_transcription_then_the_file() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let mut answers = audio_answers();
    answers.push(answer(500, r#"{"error_message":"transcript unavailable"}"#));
    answers.push(answer(200, "{}"));
    answers.push(answer(200, "{}"));
    let recorder = support::Recorder::new(answers, &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = super::support::params(&[("soniox_max_polling_attempts", json!(1))]);
    let call = call(&case_params, false);
    let mut stream = support::NamedStream::new(AUDIO_NAME, AUDIO_CONTENT);
    let mut file = AudioFile::Stream(&mut stream);
    let mut response = response();
    let mut logger = support::Logger::new(&effects, &book);
    let outcome = pipeline
        .transcribe::<support::Recorder<'_>>(
            &call,
            target(Some(&mut file), &mut response, &mut logger),
            None,
        )
        .await;

    let error = outcome.expect_err("the fetch rejection ends the job");
    let exception = error.provider().expect("the rejection is the Soniox 500");
    assert_eq!(exception.status_code, 500);
    assert_eq!(
        exception.message,
        "Soniox fetch transcript failed (HTTP 500): transcript unavailable"
    );
    let calls = recorder.calls();
    assert_eq!(calls.len(), 6);
    assert_eq!(calls[4].method, Method::DELETE);
    assert_eq!(calls[4].url, format!("{TRANSCRIPTIONS_URL}/tr_1"));
    assert_eq!(calls[5].method, Method::DELETE);
    assert_eq!(calls[5].url, "https://api.soniox.test/v1/files/file_1");
    assert_eq!(response.text.as_deref(), Some(""));
}

#[tokio::test]
async fn a_poll_error_status_reports_the_failure_and_cleans_both() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let mut answers = vec![
        answer(200, r#"{"id":"file_1"}"#),
        answer(200, r#"{"id":"tr_1"}"#),
        answer(200, r#"{"status":"error","error_message":"bad audio"}"#),
    ];
    answers.push(answer(200, "{}"));
    answers.push(answer(200, "{}"));
    let recorder = support::Recorder::new(answers, &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = super::support::params(&[("soniox_max_polling_attempts", json!(1))]);
    let call = call(&case_params, false);
    let mut stream = support::NamedStream::new(AUDIO_NAME, AUDIO_CONTENT);
    let mut file = AudioFile::Stream(&mut stream);
    let mut response = response();
    let mut logger = support::Logger::new(&effects, &book);
    let outcome = pipeline
        .transcribe::<support::Recorder<'_>>(
            &call,
            target(Some(&mut file), &mut response, &mut logger),
            None,
        )
        .await;

    let error = outcome.expect_err("a failed remote job ends the call");
    let exception = error
        .provider()
        .expect("the remote failure is a Soniox 500");
    assert_eq!(exception.status_code, 500);
    assert_eq!(
        exception.message,
        "Soniox transcription tr_1 failed: bad audio"
    );
    let calls = recorder.calls();
    assert_eq!(calls.len(), 5);
    assert_eq!(calls[3].url, format!("{TRANSCRIPTIONS_URL}/tr_1"));
    assert_eq!(calls[4].url, "https://api.soniox.test/v1/files/file_1");
}

#[tokio::test]
async fn an_invalid_duration_after_the_post_call_keeps_the_built_response() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let mut answers = vec![
        answer(200, r#"{"id":"file_1"}"#),
        answer(200, r#"{"id":"tr_1"}"#),
        answer(200, r#"{"status":"completed","audio_duration_ms":"nope"}"#),
        answer(200, r#"{"text":"hello"}"#),
    ];
    answers.push(answer(200, "{}"));
    answers.push(answer(200, "{}"));
    let recorder = support::Recorder::new(answers, &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = super::support::params(&[("soniox_max_polling_attempts", json!(1))]);
    let call = call(&case_params, false);
    let mut stream = support::NamedStream::new(AUDIO_NAME, AUDIO_CONTENT);
    let mut file = AudioFile::Stream(&mut stream);
    let mut response = response();
    let mut logger = support::Logger::new(&effects, &book);
    let outcome = pipeline
        .transcribe::<support::Recorder<'_>>(
            &call,
            target(Some(&mut file), &mut response, &mut logger),
            None,
        )
        .await;

    match outcome.expect_err("the duration cannot become seconds") {
        PipelineError::Duration { field, value } => {
            assert_eq!(field, "audio_duration_ms");
            assert_eq!(value, "\"nope\"");
        }
        other => panic!("expected the duration error, got {other:?}"),
    }
    assert_eq!(
        effects.kinds(),
        [
            "request",
            "pre_call",
            "request",
            "request",
            "request",
            "post_call",
            "request",
            "request",
        ]
    );
    assert_eq!(response.text.as_deref(), Some("hello"));
    assert_eq!(response.extra.get("task"), Some(&json!("transcribe")));
    assert_eq!(response.extra.get("duration"), None);
    assert!(response.hidden_params.contains_key("soniox_raw"));
    assert_eq!(response.hidden_params.get("model"), None);
    assert_eq!(response.hidden_params.get("custom_llm_provider"), None);
    assert_eq!(
        response.hidden_params.get("audio_transcription_duration"),
        None
    );
    let calls = recorder.calls();
    assert_eq!(calls.len(), 6);
    assert_eq!(calls[4].url, format!("{TRANSCRIPTIONS_URL}/tr_1"));
    assert_eq!(calls[5].url, "https://api.soniox.test/v1/files/file_1");
}

#[tokio::test]
async fn a_failing_delete_does_not_replace_the_transcription_result() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let mut answers = audio_answers();
    answers.push(answer(200, r#"{"text":"hello"}"#));
    answers.push(support::Reply::Fail(TransportError::Closed));
    answers.push(support::Reply::Fail(TransportError::Closed));
    let recorder = support::Recorder::new(answers, &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = super::support::params(&[("soniox_max_polling_attempts", json!(1))]);
    let call = call(&case_params, false);
    let mut stream = support::NamedStream::new(AUDIO_NAME, AUDIO_CONTENT);
    let mut file = AudioFile::Stream(&mut stream);
    let mut response = response();
    let mut logger = support::Logger::new(&effects, &book);
    let outcome = pipeline
        .transcribe::<support::Recorder<'_>>(
            &call,
            target(Some(&mut file), &mut response, &mut logger),
            None,
        )
        .await;

    assert!(outcome.is_ok(), "{outcome:?}");
    assert_eq!(response.text.as_deref(), Some("hello"));
    assert_eq!(response.hidden_params.get("model"), Some(&json!(MODEL)));
    let calls = recorder.calls();
    assert_eq!(calls.len(), 6);
    assert_eq!(calls[4].method, Method::DELETE);
    assert_eq!(calls[5].method, Method::DELETE);
}

#[tokio::test]
async fn a_malformed_create_reply_leaves_the_job_and_deletes_the_file() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let answers = vec![
        answer(200, r#"{"id":"file_1"}"#),
        answer(200, r#"{"resource":"missing"}"#),
        answer(200, "{}"),
    ];
    let recorder = support::Recorder::new(answers, &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = super::support::params(&[("soniox_max_polling_attempts", json!(1))]);
    let call = call(&case_params, false);
    let mut stream = support::NamedStream::new(AUDIO_NAME, AUDIO_CONTENT);
    let mut file = AudioFile::Stream(&mut stream);
    let mut response = response();
    let mut logger = support::Logger::new(&effects, &book);
    let outcome = pipeline
        .transcribe::<support::Recorder<'_>>(
            &call,
            target(Some(&mut file), &mut response, &mut logger),
            None,
        )
        .await;

    assert!(matches!(
        outcome.expect_err("the create reply has no object id"),
        PipelineError::Reply(_)
    ));
    let calls = recorder.calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[2].method, Method::DELETE);
    assert_eq!(calls[2].url, "https://api.soniox.test/v1/files/file_1");
    assert!(
        calls
            .iter()
            .all(|call| !call.url.contains("/transcriptions/"))
    );
}

#[tokio::test]
async fn a_transport_failure_leaves_no_error_text_in_the_response() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let answers = vec![
        answer(200, r#"{"id":"file_1"}"#),
        support::Reply::Fail(TransportError::Closed),
    ];
    let recorder = support::Recorder::new(answers, &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = super::support::params(&[("soniox_max_polling_attempts", json!(1))]);
    let call = call(&case_params, false);
    let mut stream = support::NamedStream::new(AUDIO_NAME, AUDIO_CONTENT);
    let mut file = AudioFile::Stream(&mut stream);
    let mut response = response();
    let mut logger = support::Logger::new(&effects, &book);
    let outcome = pipeline
        .transcribe::<support::Recorder<'_>>(
            &call,
            target(Some(&mut file), &mut response, &mut logger),
            None,
        )
        .await;

    assert!(matches!(
        outcome.expect_err("the closed transport ends the job"),
        PipelineError::Transport(_)
    ));
    let calls = recorder.calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[2].url, "https://api.soniox.test/v1/files/file_1");
    assert_eq!(response.text.as_deref(), Some(""));
}

#[test]
fn dropping_the_job_future_skips_the_cleanup_requests() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let answers = vec![answer(200, r#"{"id":"file_1"}"#), support::Reply::Hang];
    let recorder = support::Recorder::new(answers, &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = super::support::params(&[("soniox_max_polling_attempts", json!(1))]);
    let call = call(&case_params, false);
    let mut stream = support::NamedStream::new(AUDIO_NAME, AUDIO_CONTENT);
    let mut file = AudioFile::Stream(&mut stream);
    let mut response = response();
    let mut logger = support::Logger::new(&effects, &book);
    let job = pipeline.transcribe::<support::Recorder<'_>>(
        &call,
        target(Some(&mut file), &mut response, &mut logger),
        None,
    );
    let mut job = Box::pin(job);
    let mut context = Context::from_waker(Waker::noop());
    assert!(job.as_mut().poll(&mut context).is_pending());
    drop(job);

    assert_eq!(effects.kinds(), ["request", "pre_call", "request"]);
    let calls = recorder.calls();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|call| call.method != Method::DELETE));
    assert_eq!(response.text.as_deref(), Some(""));
}

#[tokio::test]
async fn a_numeric_string_duration_is_refused_where_python_reads_a_float() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let mut answers = vec![
        answer(200, r#"{"id":"file_1"}"#),
        answer(200, r#"{"id":"tr_1"}"#),
        answer(200, r#"{"status":"completed","audio_duration_ms":"1500"}"#),
        answer(200, r#"{"text":"hello"}"#),
    ];
    answers.push(answer(200, "{}"));
    answers.push(answer(200, "{}"));
    let recorder = support::Recorder::new(answers, &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = super::support::params(&[("soniox_max_polling_attempts", json!(1))]);
    let call = call(&case_params, false);
    let mut stream = support::NamedStream::new(AUDIO_NAME, AUDIO_CONTENT);
    let mut file = AudioFile::Stream(&mut stream);
    let mut response = response();
    let mut logger = support::Logger::new(&effects, &book);
    let outcome = pipeline
        .transcribe::<support::Recorder<'_>>(
            &call,
            target(Some(&mut file), &mut response, &mut logger),
            None,
        )
        .await;

    match outcome.expect_err("a string is not a milliseconds number") {
        PipelineError::Duration { field, value } => {
            assert_eq!(field, "audio_duration_ms");
            assert_eq!(value, "\"1500\"");
        }
        other => panic!("expected the duration error, got {other:?}"),
    }
    assert_eq!(response.text.as_deref(), Some("hello"));
    let calls = recorder.calls();
    assert_eq!(calls.len(), 6);
    assert_eq!(calls[4].method, Method::DELETE);
    assert_eq!(calls[5].method, Method::DELETE);
}

#[tokio::test]
async fn a_large_integer_duration_is_divided_once_before_rounding_to_seconds() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let mut answers = vec![
        answer(200, r#"{"id":"file_1"}"#),
        answer(200, r#"{"id":"tr_1"}"#),
        answer(
            200,
            r#"{"status":"completed","audio_duration_ms":9007199254740995}"#,
        ),
        answer(200, r#"{"text":"hello"}"#),
    ];
    answers.push(answer(200, "{}"));
    answers.push(answer(200, "{}"));
    let recorder = support::Recorder::new(answers, &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = super::support::params(&[("soniox_max_polling_attempts", json!(1))]);
    let call = call(&case_params, false);
    let mut stream = support::NamedStream::new(AUDIO_NAME, AUDIO_CONTENT);
    let mut file = AudioFile::Stream(&mut stream);
    let mut response = response();
    let mut logger = support::Logger::new(&effects, &book);
    let outcome = pipeline
        .transcribe::<support::Recorder<'_>>(
            &call,
            target(Some(&mut file), &mut response, &mut logger),
            None,
        )
        .await;

    assert!(outcome.is_ok(), "{outcome:?}");
    assert_eq!(
        response.hidden_params.get("audio_transcription_duration"),
        Some(&json!(9007199254740.994))
    );
}

#[tokio::test]
async fn a_boolean_duration_is_refused_where_python_reads_a_float() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let mut answers = vec![
        answer(200, r#"{"id":"file_1"}"#),
        answer(200, r#"{"id":"tr_1"}"#),
        answer(200, r#"{"status":"completed","audio_duration_ms":true}"#),
        answer(200, r#"{"text":"hello"}"#),
    ];
    answers.push(answer(200, "{}"));
    answers.push(answer(200, "{}"));
    let recorder = support::Recorder::new(answers, &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = super::support::params(&[("soniox_max_polling_attempts", json!(1))]);
    let call = call(&case_params, false);
    let mut stream = support::NamedStream::new(AUDIO_NAME, AUDIO_CONTENT);
    let mut file = AudioFile::Stream(&mut stream);
    let mut response = response();
    let mut logger = support::Logger::new(&effects, &book);
    let outcome = pipeline
        .transcribe::<support::Recorder<'_>>(
            &call,
            target(Some(&mut file), &mut response, &mut logger),
            None,
        )
        .await;

    match outcome.expect_err("a boolean is not a milliseconds number") {
        PipelineError::Duration { field, value } => {
            assert_eq!(field, "audio_duration_ms");
            assert_eq!(value, "true");
        }
        other => panic!("expected the duration error, got {other:?}"),
    }
    assert_eq!(response.text.as_deref(), Some("hello"));
    let calls = recorder.calls();
    assert_eq!(calls.len(), 6);
    assert_eq!(calls[4].method, Method::DELETE);
    assert_eq!(calls[5].method, Method::DELETE);
}

#[tokio::test]
async fn a_bad_api_base_fails_the_create_without_a_request() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let recorder = support::Recorder::new(Vec::new(), &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = super::support::params(&[
        ("soniox_max_polling_attempts", json!(1)),
        ("file_id", json!("caller-file")),
    ]);
    let mut call = call(&case_params, false);
    call.api_base = Some("not a url");
    let mut response = response();
    let mut logger = support::Logger::new(&effects, &book);
    let outcome = pipeline
        .transcribe::<support::Recorder<'_>>(&call, target(None, &mut response, &mut logger), None)
        .await;

    match outcome.expect_err("the create URL cannot be parsed") {
        PipelineError::Url { url, .. } => {
            assert_eq!(url, "not a url/v1/transcriptions");
        }
        other => panic!("expected the URL error, got {other:?}"),
    }
    assert_eq!(effects.kinds(), ["pre_call"]);
    assert!(recorder.calls().is_empty());
}

#[tokio::test]
async fn an_unsendable_header_fails_the_create_without_a_request() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let recorder = support::Recorder::new(Vec::new(), &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = super::support::params(&[
        ("soniox_max_polling_attempts", json!(1)),
        ("file_id", json!("caller-file")),
    ]);
    let mut call = call(&case_params, false);
    call.headers
        .insert("bad name".to_owned(), "value".to_owned());
    let mut response = response();
    let mut logger = support::Logger::new(&effects, &book);
    let outcome = pipeline
        .transcribe::<support::Recorder<'_>>(&call, target(None, &mut response, &mut logger), None)
        .await;

    assert!(matches!(
        outcome.expect_err("the header cannot be sent"),
        PipelineError::Header(_)
    ));
    assert_eq!(effects.kinds(), ["pre_call"]);
    assert!(recorder.calls().is_empty());
}
