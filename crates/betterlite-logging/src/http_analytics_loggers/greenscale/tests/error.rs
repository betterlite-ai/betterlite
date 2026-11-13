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
async fn greenscale_swallows_a_delivery_failure_and_logs_it() {
    let captured = Captured::default();
    let _guard = captured.subscribe();
    let capture = Capture::with_answers([Answer::Fail]);
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
    assert!(captured.text().contains("Greenscale logger error"));
}

#[tokio::test]
async fn greenscale_swallows_a_missing_endpoint_and_logs_it() {
    let captured = Captured::default();
    let _guard = captured.subscribe();
    let capture = Capture::default();
    let clock = FixedClock {
        now: time("2026-01-02T03:04:05Z"),
    };
    let logger = GreenscaleLogger {
        delivery: capture.clone(),
        clock,
        settings: GreenscaleSettings::from_environment(&environment(&[(
            "GREENSCALE_API_KEY",
            "green-key",
        )])),
    };
    let response = json(COMPLETION);
    logger
        .log_event(&event(&json(KWARGS), Some(&response), START, END))
        .await;
    assert!(capture.requests().is_empty());
    assert!(captured.text().contains("GREENSCALE_ENDPOINT"));
}

#[tokio::test]
async fn greenscale_swallows_a_missing_api_key_and_logs_it() {
    let captured = Captured::default();
    let _guard = captured.subscribe();
    let capture = Capture::default();
    let clock = FixedClock {
        now: time("2026-01-02T03:04:05Z"),
    };
    let logger = GreenscaleLogger {
        delivery: capture.clone(),
        clock,
        settings: GreenscaleSettings::from_environment(&environment(&[(
            "GREENSCALE_ENDPOINT",
            "https://greenscale.example.com/v1/log",
        )])),
    };
    let response = json(COMPLETION);
    logger
        .log_event(&event(&json(KWARGS), Some(&response), START, END))
        .await;
    assert!(capture.requests().is_empty());
    assert!(captured.text().contains("GREENSCALE_API_KEY"));
}
