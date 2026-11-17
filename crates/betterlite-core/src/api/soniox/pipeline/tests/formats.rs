use http::Method;
use serde_json::json;

use super::support::{
    self, MODEL, TRANSCRIPTIONS_URL, answer, call, json_body, params, pipeline_with, response,
    target,
};

const TIMED_TRANSCRIPT: &str = concat!(
    r#"{"text":"Hello","#,
    r#""tokens":[{"text":"He","start_ms":0,"end_ms":500},"#,
    r#"{"text":"llo","start_ms":500,"end_ms":1000}],"#,
    r#""audio_duration_ms":1000}"#
);

#[tokio::test]
async fn an_srt_response_format_renders_the_transcript_as_subtitles() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let answers = vec![
        answer(200, r#"{"id":"tr_1"}"#),
        answer(200, r#"{"status":"completed","audio_duration_ms":1000}"#),
        answer(200, TIMED_TRANSCRIPT),
        answer(200, "{}"),
    ];
    let recorder = support::Recorder::new(answers, &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = params(&[
        ("soniox_max_polling_attempts", json!(1)),
        ("file_id", json!("caller-file")),
        ("response_format", json!("srt")),
    ]);
    let call = call(&case_params, false);
    let mut response = response();
    let mut logger = support::Logger::new(&effects, &book);
    let outcome = pipeline
        .transcribe::<support::Recorder<'_>>(&call, target(None, &mut response, &mut logger), None)
        .await;

    assert!(outcome.is_ok(), "{outcome:?}");
    assert_eq!(
        response.text.as_deref(),
        Some("1\n00:00:00,000 --> 00:00:01,000\nHello\n")
    );
    assert!(!response.extra.contains_key("words"));
    let calls = recorder.calls();
    assert_eq!(
        json_body(&calls[0].body),
        &json!({"model": MODEL, "file_id": "caller-file"})
    );
    assert_eq!(calls[3].method, Method::DELETE);
    assert_eq!(calls[3].url, format!("{TRANSCRIPTIONS_URL}/tr_1"));
}

#[tokio::test]
async fn a_verbose_json_response_format_adds_the_word_timings() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let answers = vec![
        answer(200, r#"{"id":"tr_1"}"#),
        answer(200, r#"{"status":"completed","audio_duration_ms":1000}"#),
        answer(200, TIMED_TRANSCRIPT),
        answer(200, "{}"),
    ];
    let recorder = support::Recorder::new(answers, &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = params(&[
        ("soniox_max_polling_attempts", json!(1)),
        ("file_id", json!("caller-file")),
        ("response_format", json!("verbose_json")),
    ]);
    let call = call(&case_params, false);
    let mut response = response();
    let mut logger = support::Logger::new(&effects, &book);
    let outcome = pipeline
        .transcribe::<support::Recorder<'_>>(&call, target(None, &mut response, &mut logger), None)
        .await;

    assert!(outcome.is_ok(), "{outcome:?}");
    assert_eq!(response.text.as_deref(), Some("Hello"));
    assert_eq!(
        response.extra.get("words"),
        Some(&json!([
            {"word": "He", "start": 0.0, "end": 0.5},
            {"word": "llo", "start": 0.5, "end": 1.0},
        ]))
    );
    let calls = recorder.calls();
    assert_eq!(
        json_body(&calls[0].body),
        &json!({"model": MODEL, "file_id": "caller-file"})
    );
}
