use std::path::Path;
use std::time::Duration;

use betterlite_openai::api::audio_file_input::{AudioFile, AudioTuple, TupleContent};
use http::Method;
use serde_json::json;

use super::support::{
    self, AUDIO_CONTENT, AUDIO_NAME, KEY, MODEL, TIMEOUT, TRANSCRIPTIONS_URL, UPLOAD_URL, answer,
    call, json_body, multipart_parts, params, pipeline_with, response, sent, target,
};
use crate::api::soniox::pipeline::error::PipelineError;
use crate::api::soniox::pipeline::job::MISSING_INPUT;

#[tokio::test]
async fn uploaded_success_runs_create_poll_fetch_and_deletes_both() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let answers = vec![
        answer(200, r#"{"id":"file_1"}"#),
        answer(200, r#"{"id":"tr_1"}"#),
        answer(200, r#"{"status":"completed","audio_duration_ms":1500}"#),
        answer(
            200,
            r#"{"text":"hello","tokens":[],"audio_duration_ms":1500}"#,
        ),
        answer(200, "{}"),
        answer(200, "{}"),
    ];
    let recorder = support::Recorder::new(answers, &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = params(&[("soniox_max_polling_attempts", json!(1))]);
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
    let calls = recorder.calls();
    assert_eq!(calls.len(), 6);
    assert_eq!(calls[0].method, Method::POST);
    assert_eq!(calls[0].url, UPLOAD_URL);
    let bearer = format!("Bearer {KEY}");
    assert_eq!(sent(&calls[0], "authorization"), Some(bearer.as_str()));
    assert_eq!(sent(&calls[0], "x-client"), None);
    let parts = multipart_parts(&calls[0].body);
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].name, "file");
    assert_eq!(parts[0].file_name.as_deref(), Some(AUDIO_NAME));
    assert_eq!(parts[0].content_type.as_deref(), Some("audio/mpeg"));
    assert_eq!(parts[0].data, AUDIO_CONTENT);
    assert_eq!(calls[1].method, Method::POST);
    assert_eq!(calls[1].url, TRANSCRIPTIONS_URL);
    assert_eq!(sent(&calls[1], "x-client"), Some("suite"));
    assert_eq!(
        json_body(&calls[1].body),
        &json!({"model": MODEL, "file_id": "file_1"})
    );
    assert_eq!(calls[2].method, Method::GET);
    assert_eq!(calls[2].url, format!("{TRANSCRIPTIONS_URL}/tr_1"));
    assert_eq!(calls[3].method, Method::GET);
    assert_eq!(
        calls[3].url,
        format!("{TRANSCRIPTIONS_URL}/tr_1/transcript")
    );
    assert_eq!(calls[4].method, Method::DELETE);
    assert_eq!(calls[4].url, format!("{TRANSCRIPTIONS_URL}/tr_1"));
    assert_eq!(calls[5].method, Method::DELETE);
    assert_eq!(calls[5].url, "https://api.soniox.test/v1/files/file_1");
    assert!(calls.iter().all(|call| call.deadline == TIMEOUT));
    assert_eq!(response.text.as_deref(), Some("hello"));
    assert_eq!(response.extra.get("task"), Some(&json!("transcribe")));
    assert_eq!(response.extra.get("duration"), Some(&json!(1.5)));
    assert_eq!(response.hidden_params.get("model"), Some(&json!(MODEL)));
    assert_eq!(
        response.hidden_params.get("custom_llm_provider"),
        Some(&json!("soniox"))
    );
    assert_eq!(
        response.hidden_params.get("audio_transcription_duration"),
        Some(&json!(1.5))
    );
    let records = book.records();
    let [pre, post] = records.as_slice() else {
        panic!("the job logs one pre-call and one post-call record")
    };
    match pre {
        support::LogRecord::PreCall { url, body } => {
            assert_eq!(url, TRANSCRIPTIONS_URL);
            assert_eq!(body, &json!({"model": MODEL, "file_id": "file_1"}));
        }
        support::LogRecord::PostCall { .. } => panic!("the first record is the pre-call"),
    }
    match post {
        support::LogRecord::PostCall { input, original } => {
            assert_eq!(input, &json!(AUDIO_NAME));
            assert_eq!(
                original,
                &json!({
                    "transcription": {"status": "completed", "audio_duration_ms": 1500},
                    "transcript": {"text": "hello", "tokens": [], "audio_duration_ms": 1500},
                })
            );
        }
        support::LogRecord::PreCall { .. } => panic!("the second record is the post-call"),
    }
}

#[tokio::test]
async fn a_tuple_audio_file_uploads_its_streamed_content_with_the_tuple_filename() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let answers = vec![
        answer(200, r#"{"id":"file_1"}"#),
        answer(200, r#"{"id":"tr_1"}"#),
        answer(200, r#"{"status":"completed","audio_duration_ms":1500}"#),
        answer(200, r#"{"text":"hello"}"#),
        answer(200, "{}"),
        answer(200, "{}"),
    ];
    let recorder = support::Recorder::new(answers, &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = params(&[("soniox_max_polling_attempts", json!(1))]);
    let call = call(&case_params, false);
    let mut stream = support::NamedStream::new("inner.mp3", AUDIO_CONTENT);
    let mut file = AudioFile::Tuple(AudioTuple {
        filename: "clip.mp3",
        content: Some(TupleContent::Stream(&mut stream)),
    });
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
    let calls = recorder.calls();
    let parts = multipart_parts(&calls[0].body);
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].file_name.as_deref(), Some("clip.mp3"));
    assert_eq!(parts[0].content_type.as_deref(), Some("audio/mpeg"));
    assert_eq!(parts[0].data, AUDIO_CONTENT);
    assert_eq!(response.text.as_deref(), Some("hello"));
}

#[tokio::test]
async fn a_caller_file_is_never_uploaded_or_deleted() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let answers = vec![
        answer(200, r#"{"id":"tr_1"}"#),
        answer(200, r#"{"status":"completed","audio_duration_ms":1500}"#),
        answer(200, r#"{"text":"hello"}"#),
        answer(200, "{}"),
    ];
    let recorder = support::Recorder::new(answers, &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = params(&[
        ("soniox_max_polling_attempts", json!(1)),
        ("file_id", json!("caller-file")),
    ]);
    let call = call(&case_params, false);
    let mut response = response();
    let mut logger = support::Logger::new(&effects, &book);
    let outcome = pipeline
        .transcribe::<support::Recorder<'_>>(&call, target(None, &mut response, &mut logger), None)
        .await;

    assert!(outcome.is_ok(), "{outcome:?}");
    assert_eq!(
        effects.kinds(),
        [
            "pre_call",
            "request",
            "request",
            "request",
            "post_call",
            "request",
        ]
    );
    let calls = recorder.calls();
    assert_eq!(calls.len(), 4);
    assert_eq!(calls[0].url, TRANSCRIPTIONS_URL);
    assert_eq!(
        json_body(&calls[0].body),
        &json!({"model": MODEL, "file_id": "caller-file"})
    );
    assert_eq!(calls[3].method, Method::DELETE);
    assert_eq!(calls[3].url, format!("{TRANSCRIPTIONS_URL}/tr_1"));
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.method == Method::DELETE)
            .count(),
        1
    );
    assert!(calls.iter().all(|call| call.url != UPLOAD_URL));
    assert_eq!(response.text.as_deref(), Some("hello"));
}

#[tokio::test]
async fn an_audio_url_skips_the_upload_and_still_deletes_the_job() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let answers = vec![
        answer(200, r#"{"id":"tr_1"}"#),
        answer(200, r#"{"status":"completed","audio_duration_ms":1500}"#),
        answer(200, r#"{"text":"hello"}"#),
        answer(200, "{}"),
    ];
    let recorder = support::Recorder::new(answers, &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = params(&[
        ("soniox_max_polling_attempts", json!(1)),
        ("audio_url", json!("https://audio.test/a.mp3")),
    ]);
    let call = call(&case_params, false);
    let mut response = response();
    let mut logger = support::Logger::new(&effects, &book);
    let outcome = pipeline
        .transcribe::<support::Recorder<'_>>(&call, target(None, &mut response, &mut logger), None)
        .await;

    assert!(outcome.is_ok(), "{outcome:?}");
    let calls = recorder.calls();
    assert!(calls.iter().all(|call| call.url != UPLOAD_URL));
    assert_eq!(
        json_body(&calls[0].body),
        &json!({"model": MODEL, "audio_url": "https://audio.test/a.mp3"})
    );
    assert_eq!(calls.len(), 4);
    assert_eq!(calls[3].method, Method::DELETE);
    assert_eq!(calls[3].url, format!("{TRANSCRIPTIONS_URL}/tr_1"));
}

#[tokio::test]
async fn both_references_are_sent_in_the_create_body() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let answers = vec![
        answer(200, r#"{"id":"tr_1"}"#),
        answer(200, r#"{"status":"completed","audio_duration_ms":1500}"#),
        answer(200, r#"{"text":"hello"}"#),
        answer(200, "{}"),
    ];
    let recorder = support::Recorder::new(answers, &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = params(&[
        ("soniox_max_polling_attempts", json!(1)),
        ("file_id", json!("caller-file")),
        ("audio_url", json!("https://audio.test/a.mp3")),
    ]);
    let call = call(&case_params, false);
    let mut response = response();
    let mut logger = support::Logger::new(&effects, &book);
    let outcome = pipeline
        .transcribe::<support::Recorder<'_>>(&call, target(None, &mut response, &mut logger), None)
        .await;

    assert!(outcome.is_ok(), "{outcome:?}");
    let calls = recorder.calls();
    assert!(calls.iter().all(|call| call.url != UPLOAD_URL));
    assert_eq!(
        json_body(&calls[0].body),
        &json!({
            "model": MODEL,
            "file_id": "caller-file",
            "audio_url": "https://audio.test/a.mp3",
        })
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.method == Method::DELETE)
            .count(),
        1
    );
}

#[tokio::test]
async fn missing_input_raises_the_pinned_400_without_a_request() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let recorder = support::Recorder::new(Vec::new(), &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = params(&[]);
    let call = call(&case_params, false);
    let mut response = response();
    let mut logger = support::Logger::new(&effects, &book);
    let outcome = pipeline
        .transcribe::<support::Recorder<'_>>(&call, target(None, &mut response, &mut logger), None)
        .await;

    let error = outcome.expect_err("a call with no audio reference is rejected");
    let exception = error.provider().expect("the rejection is the Soniox 400");
    assert_eq!(exception.status_code, 400);
    assert_eq!(exception.message, MISSING_INPUT);
    assert_eq!(exception.headers, None);
    assert!(recorder.calls().is_empty());
    assert!(effects.kinds().is_empty());
    assert_eq!(response.text.as_deref(), Some(""));
    assert!(response.hidden_params.is_empty());
}

#[tokio::test]
async fn an_upload_failure_cleans_nothing_and_keeps_the_provider_error() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let answers = vec![answer(500, r#"{"error_message":"no space"}"#)];
    let recorder = support::Recorder::new(answers, &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = params(&[("soniox_max_polling_attempts", json!(1))]);
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

    let error = outcome.expect_err("the upload rejection ends the job");
    let exception = error.provider().expect("the rejection is the Soniox 500");
    assert_eq!(exception.status_code, 500);
    assert_eq!(
        exception.message,
        "Soniox upload file failed (HTTP 500): no space"
    );
    let calls = recorder.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].url, UPLOAD_URL);
    assert_eq!(effects.kinds(), ["request"]);
    assert_eq!(response.text.as_deref(), Some(""));
}

#[tokio::test]
async fn an_unreadable_audio_path_reports_the_audio_error_without_a_request() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let recorder = support::Recorder::new(Vec::new(), &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = params(&[("soniox_max_polling_attempts", json!(1))]);
    let call = call(&case_params, false);
    let mut file = AudioFile::Path(Path::new("/missing/audio.mp3"));
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
        outcome.expect_err("the file cannot be read"),
        PipelineError::Audio(_)
    ));
    assert!(recorder.calls().is_empty());
    assert!(effects.kinds().is_empty());
}

#[tokio::test]
async fn an_empty_file_reference_falls_through_to_the_missing_input_error() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let recorder = support::Recorder::new(Vec::new(), &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = params(&[("file_id", json!("")), ("audio_url", json!(""))]);
    let call = call(&case_params, false);
    let mut response = response();
    let mut logger = support::Logger::new(&effects, &book);
    let outcome = pipeline
        .transcribe::<support::Recorder<'_>>(&call, target(None, &mut response, &mut logger), None)
        .await;

    let error = outcome.expect_err("an empty reference is not a reference");
    let exception = error.provider().expect("the rejection is the Soniox 400");
    assert_eq!(exception.status_code, 400);
    assert_eq!(exception.message, MISSING_INPUT);
    assert!(recorder.calls().is_empty());
}

#[tokio::test]
async fn a_completed_poll_without_a_duration_leaves_the_hidden_value_null() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let mut answers = vec![
        answer(200, r#"{"id":"file_1"}"#),
        answer(200, r#"{"id":"tr_1"}"#),
        answer(200, r#"{"status":"completed"}"#),
        answer(200, r#"{"text":"hello"}"#),
    ];
    answers.push(answer(200, "{}"));
    answers.push(answer(200, "{}"));
    let recorder = support::Recorder::new(answers, &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = params(&[("soniox_max_polling_attempts", json!(1))]);
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
        Some(&json!(null))
    );
    assert_eq!(response.hidden_params.get("model"), Some(&json!(MODEL)));
}

#[tokio::test]
async fn a_scientific_duration_is_read_as_its_exact_decimal() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let mut answers = vec![
        answer(200, r#"{"id":"file_1"}"#),
        answer(200, r#"{"id":"tr_1"}"#),
        answer(200, r#"{"status":"completed","audio_duration_ms":1e-7}"#),
        answer(200, r#"{"text":"hello"}"#),
    ];
    answers.push(answer(200, "{}"));
    answers.push(answer(200, "{}"));
    let recorder = support::Recorder::new(answers, &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = params(&[("soniox_max_polling_attempts", json!(1))]);
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
        Some(&json!(1e-10))
    );
}

#[tokio::test]
async fn a_pending_poll_sleeps_the_poll_interval_between_attempts() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let answers = vec![
        answer(200, r#"{"id":"file_1"}"#),
        answer(200, r#"{"id":"tr_1"}"#),
        answer(200, r#"{"status":"running"}"#),
        answer(200, r#"{"status":"completed","audio_duration_ms":1500}"#),
        answer(200, r#"{"text":"hello"}"#),
        answer(200, "{}"),
        answer(200, "{}"),
    ];
    let recorder = support::Recorder::new(answers, &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = params(&[("soniox_max_polling_attempts", json!(2))]);
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
    assert_eq!(recorder.sleeps(), [Duration::from_millis(1000)]);
    assert_eq!(
        effects.kinds(),
        [
            "request",
            "pre_call",
            "request",
            "request",
            "sleep",
            "request",
            "request",
            "post_call",
            "request",
            "request",
        ]
    );
    assert_eq!(response.text.as_deref(), Some("hello"));
}
