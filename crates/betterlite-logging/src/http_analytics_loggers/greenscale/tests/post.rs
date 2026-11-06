use crate::http_analytics_loggers::greenscale::{GreenscaleLogger, GreenscaleSettings};
use crate::http_analytics_loggers::tests::support::{
    Answer, COMPLETION, Capture, Captured, FixedClock, environment, event, json, time,
};

const START: &str = "2026-01-02T03:00:00Z";
const END: &str = "2026-01-02T03:00:01Z";
const ENVIRONMENT: &[(&str, &str)] = &[
    ("GREENSCALE_API_KEY", "green-key"),
    (
        "GREENSCALE_ENDPOINT",
        "https://greenscale.example.com/v1/log",
    ),
];
const KWARGS: &str = r#"{"model":"gpt-4o","litellm_params":{"metadata":{}}}"#;

#[tokio::test]
async fn greenscale_posts_to_the_configured_endpoint_with_the_key_header() {
    let capture = Capture::default();
    let clock = FixedClock {
        now: time("2026-01-02T03:04:05Z"),
    };
    let logger = GreenscaleLogger {
        delivery: capture.clone(),
        clock,
        settings: GreenscaleSettings::from_environment(&environment(ENVIRONMENT)),
    };
    let response = json(COMPLETION);
    logger
        .log_event(&event(&json(KWARGS), Some(&response), START, END))
        .await;
    let requests = capture.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, "https://greenscale.example.com/v1/log");
    assert_eq!(
        requests[0].headers.get("api-key").map(String::as_str),
        Some("green-key")
    );
    assert_eq!(
        requests[0].headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
}

#[rstest::rstest]
#[case::success(200, "Greenscale logger succeeded")]
#[case::server_error(500, "Greenscale logger error")]
#[tokio::test]
async fn greenscale_logs_every_status_without_failing(#[case] status: u16, #[case] message: &str) {
    let captured = Captured::default();
    let _guard = captured.subscribe();
    let capture = Capture::with_answers([Answer::Status(status)]);
    let clock = FixedClock {
        now: time("2026-01-02T03:04:05Z"),
    };
    let logger = GreenscaleLogger {
        delivery: capture.clone(),
        clock,
        settings: GreenscaleSettings::from_environment(&environment(ENVIRONMENT)),
    };
    let response = json(COMPLETION);
    logger
        .log_event(&event(&json(KWARGS), Some(&response), START, END))
        .await;
    assert_eq!(capture.requests().len(), 1);
    assert!(captured.text().contains(message));
}
