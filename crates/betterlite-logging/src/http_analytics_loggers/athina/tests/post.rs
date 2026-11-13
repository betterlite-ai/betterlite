use crate::http_analytics_loggers::athina::{AthinaLogger, AthinaSettings};
use crate::http_analytics_loggers::tests::support::{
    Answer, COMPLETION, Capture, Captured, environment, event, json,
};

const START: &str = "2026-01-02T03:00:00Z";
const END: &str = "2026-01-02T03:00:01Z";
const KEY: &[(&str, &str)] = &[("ATHINA_API_KEY", "athina-key-1")];
const KWARGS: &str = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"Hello"}],"litellm_params":{"metadata":{}}}"#;

#[tokio::test]
async fn athina_posts_to_the_inference_endpoint_with_the_key_header() {
    let capture = Capture::default();
    let logger = AthinaLogger {
        delivery: capture.clone(),
        settings: AthinaSettings::from_environment(&environment(KEY)),
    };
    let response = json(COMPLETION);
    logger
        .log_event(&event(&json(KWARGS), Some(&response), START, END))
        .await;
    let requests = capture.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].url,
        "https://log.athina.ai/api/v1/log/inference"
    );
    assert_eq!(
        requests[0]
            .headers
            .get("athina-api-key")
            .map(String::as_str),
        Some("athina-key-1")
    );
    assert_eq!(
        requests[0].headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
}

#[rstest::rstest]
#[case::success(200, "Athina logger succeeded")]
#[case::created(201, "Athina logger error")]
#[case::server_error(500, "Athina logger error")]
#[tokio::test]
async fn athina_logs_every_status_without_failing(#[case] status: u16, #[case] message: &str) {
    let captured = Captured::default();
    let _guard = captured.subscribe();
    let capture = Capture::with_answers([Answer::Status(status)]);
    let logger = AthinaLogger {
        delivery: capture.clone(),
        settings: AthinaSettings::from_environment(&environment(KEY)),
    };
    let response = json(COMPLETION);
    logger
        .log_event(&event(&json(KWARGS), Some(&response), START, END))
        .await;
    assert_eq!(capture.requests().len(), 1);
    assert!(captured.text().contains(message));
}
