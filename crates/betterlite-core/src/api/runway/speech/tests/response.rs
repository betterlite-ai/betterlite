use http::header::CONTENT_TYPE;
use rstest::rstest;

use super::super::error::RunwaySpeechError;
use super::support::{Reply, fetch, header_text, request_headers, settings, submission};

#[tokio::test]
async fn a_completed_submission_is_polled_anyway() {
    let (answer, requests, pauses) = fetch(
        &settings(&[]),
        &[
            Reply::json(
                200,
                r#"{"id":"task_done","status":"SUCCEEDED","output":["https://delivery.test/speech.mp3"]}"#,
            ),
            Reply::audio("audio/mpeg", "audio-bytes"),
        ],
        &submission(200, r#"{"id":"task_done","status":"SUCCEEDED"}"#),
        &request_headers(&[
            ("Authorization", "Bearer sk-request"),
            ("X-Runway-Version", "2025-03-03"),
            ("Content-Type", "application/json"),
        ]),
    )
    .await;
    let content = answer.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].0,
        "https://api.dev.runwayml.com/v1/tasks/task_done"
    );
    assert_eq!(requests[1].0, "https://delivery.test/speech.mp3");
    assert_eq!(requests[0].1.len(), 2);
    assert_eq!(
        header_text(&requests[0].1, "authorization"),
        "Bearer sk-request"
    );
    assert_eq!(
        header_text(&requests[0].1, "x-runway-version"),
        "2025-03-03"
    );
    assert!(requests[0].1.get(CONTENT_TYPE).is_none());
    assert!(requests[1].1.is_empty());
    assert_eq!(content.status, 200);
    assert_eq!(
        content.headers.get("content-type").map(String::as_str),
        Some("audio/mpeg")
    );
    assert_eq!(content.body, b"audio-bytes".to_vec());
    assert!(pauses.is_empty());
}

#[tokio::test]
async fn polling_ignores_the_configured_base() {
    let loaded = settings(&[("RUNWAYML_API_BASE", "https://proxy.example.test")]);
    let (answer, requests, _) = fetch(
        &loaded,
        &[
            Reply::json(
                200,
                r#"{"id":"task_1","status":"SUCCEEDED","output":["https://delivery.test/a.mp3"]}"#,
            ),
            Reply::audio("audio/mpeg", "audio"),
        ],
        &submission(200, r#"{"id":"task_1","status":"PENDING"}"#),
        &request_headers(&[("Authorization", "Bearer k")]),
    )
    .await;
    answer.unwrap();
    assert_eq!(
        requests[0].0,
        "https://api.dev.runwayml.com/v1/tasks/task_1"
    );
}

#[tokio::test]
async fn polling_takes_the_configured_version_when_the_request_has_none() {
    let loaded = settings(&[("RUNWAYML_DEFAULT_API_VERSION", "2030-01-01")]);
    let (answer, requests, _) = fetch(
        &loaded,
        &[
            Reply::json(
                200,
                r#"{"id":"task_2","status":"SUCCEEDED","output":["https://delivery.test/a.mp3"]}"#,
            ),
            Reply::audio("audio/mpeg", "audio"),
        ],
        &submission(200, r#"{"id":"task_2","status":"PENDING"}"#),
        &request_headers(&[("Authorization", "Bearer k")]),
    )
    .await;
    answer.unwrap();
    assert_eq!(
        header_text(&requests[0].1, "x-runway-version"),
        "2030-01-01"
    );
}

#[tokio::test]
async fn a_request_without_authorization_polls_with_an_empty_one() {
    let (answer, requests, _) = fetch(
        &settings(&[]),
        &[
            Reply::json(
                200,
                r#"{"id":"task_3","status":"SUCCEEDED","output":["https://delivery.test/a.mp3"]}"#,
            ),
            Reply::audio("audio/mpeg", "audio"),
        ],
        &submission(200, r#"{"id":"task_3","status":"PENDING"}"#),
        &request_headers(&[("X-Runway-Version", "2025-03-03")]),
    )
    .await;
    answer.unwrap();
    assert_eq!(header_text(&requests[0].1, "authorization"), "");
    assert_eq!(
        header_text(&requests[0].1, "x-runway-version"),
        "2025-03-03"
    );
}

#[tokio::test]
async fn a_lowercase_request_header_is_still_found() {
    let (answer, requests, _) = fetch(
        &settings(&[]),
        &[
            Reply::json(
                200,
                r#"{"id":"task_4","status":"SUCCEEDED","output":["https://delivery.test/a.mp3"]}"#,
            ),
            Reply::audio("audio/mpeg", "audio"),
        ],
        &submission(200, r#"{"id":"task_4","status":"PENDING"}"#),
        &request_headers(&[
            ("authorization", "Bearer lower"),
            ("x-runway-version", "2025-05-05"),
        ]),
    )
    .await;
    answer.unwrap();
    let headers = &requests[0].1;
    assert_eq!(header_text(headers, "authorization"), "Bearer lower");
    assert_eq!(header_text(headers, "x-runway-version"), "2025-05-05");
}

#[rstest]
#[case::absent(r#"{"status":"PENDING"}"#)]
#[case::empty(r#"{"id":"","status":"PENDING"}"#)]
#[tokio::test]
async fn a_submission_without_a_task_id_fails_before_polling(#[case] body: &'static str) {
    let (answer, requests, _) = fetch(
        &settings(&[]),
        &[],
        &submission(200, body),
        &request_headers(&[("Authorization", "Bearer k")]),
    )
    .await;
    let error = answer.unwrap_err();
    assert!(matches!(error, RunwaySpeechError::MissingTaskId));
    assert_eq!(error.submission_status(), None);
    assert!(requests.is_empty());
}

#[tokio::test]
async fn a_submission_that_is_not_a_task_keeps_its_status() {
    let (answer, requests, _) = fetch(
        &settings(&[]),
        &[],
        &submission(502, "bad gateway"),
        &request_headers(&[("Authorization", "Bearer k")]),
    )
    .await;
    let error = answer.unwrap_err();
    assert_eq!(error.submission_status(), Some(502));
    assert_eq!(
        error.to_string(),
        "the RunwayML speech submission answer is not a task"
    );
    let RunwaySpeechError::Submission {
        status, headers, ..
    } = error
    else {
        panic!("a non-JSON submission is a submission failure");
    };
    assert_eq!(status, 502);
    assert!(headers.is_empty());
    assert!(requests.is_empty());
}

#[rstest]
#[case::absent(r#"{"id":"t","status":"SUCCEEDED"}"#)]
#[case::empty(r#"{"id":"t","status":"SUCCEEDED","output":[]}"#)]
#[case::text(r#"{"id":"t","status":"SUCCEEDED","output":"https://delivery.test/a.mp3"}"#)]
#[case::not_a_list(
    r#"{"id":"t","status":"SUCCEEDED","output":{"url":"https://delivery.test/a.mp3"}}"#
)]
#[tokio::test]
async fn an_output_without_an_audio_url_fails(#[case] poll: &'static str) {
    let (answer, requests, _) = fetch(
        &settings(&[]),
        &[Reply::json(200, poll)],
        &submission(200, r#"{"id":"t","status":"PENDING"}"#),
        &request_headers(&[("Authorization", "Bearer k")]),
    )
    .await;
    let error = answer.unwrap_err();
    assert!(matches!(error, RunwaySpeechError::MissingAudioUrl));
    assert_eq!(requests.len(), 1);
}

#[tokio::test]
async fn a_non_string_first_output_fails_even_with_a_later_url() {
    let (answer, requests, _) = fetch(
        &settings(&[]),
        &[Reply::json(
            200,
            r#"{"id":"t","status":"SUCCEEDED","output":[7,"https://delivery.test/a.mp3"]}"#,
        )],
        &submission(200, r#"{"id":"t","status":"PENDING"}"#),
        &request_headers(&[("Authorization", "Bearer k")]),
    )
    .await;
    let error = answer.unwrap_err();
    assert!(matches!(error, RunwaySpeechError::AudioUrlNotString));
    assert_eq!(requests.len(), 1);
}

#[tokio::test]
async fn only_the_first_output_url_is_fetched() {
    let (answer, requests, pauses) = fetch(
        &settings(&[]),
        &[
            Reply::json(200, r#"{"id":"t","status":"RUNNING"}"#),
            Reply::json(
                200,
                r#"{"id":"t","status":"SUCCEEDED","output":["https://delivery.test/first.mp3","https://delivery.test/second.mp3"]}"#,
            ),
            Reply::audio("audio/mpeg", "first-bytes"),
        ],
        &submission(200, r#"{"id":"t","status":"PENDING"}"#),
        &request_headers(&[("Authorization", "Bearer k")]),
    )
    .await;
    let content = answer.unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[2].0, "https://delivery.test/first.mp3");
    assert_eq!(pauses, [2]);
    assert_eq!(content.body, b"first-bytes".to_vec());
}

#[tokio::test]
async fn a_download_failure_reports_its_status() {
    let (answer, requests, _) = fetch(
        &settings(&[]),
        &[
            Reply::json(
                200,
                r#"{"id":"t","status":"SUCCEEDED","output":["https://delivery.test/a.mp3"]}"#,
            ),
            Reply::json(404, "missing"),
        ],
        &submission(200, r#"{"id":"t","status":"PENDING"}"#),
        &request_headers(&[("Authorization", "Bearer k")]),
    )
    .await;
    let error = answer.unwrap_err();
    assert!(matches!(
        error,
        RunwaySpeechError::DownloadStatus { status: 404 }
    ));
    assert_eq!(error.submission_status(), None);
    assert_eq!(requests.len(), 2);
}

#[tokio::test]
async fn an_audio_url_that_is_not_a_url_fails() {
    let (answer, requests, _) = fetch(
        &settings(&[]),
        &[Reply::json(
            200,
            r#"{"id":"t","status":"SUCCEEDED","output":["not a url"]}"#,
        )],
        &submission(200, r#"{"id":"t","status":"PENDING"}"#),
        &request_headers(&[("Authorization", "Bearer k")]),
    )
    .await;
    let error = answer.unwrap_err();
    assert!(matches!(error, RunwaySpeechError::AudioUrl(_)));
    assert_eq!(requests.len(), 1);
}
