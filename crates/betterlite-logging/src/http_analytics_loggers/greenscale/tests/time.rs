use crate::http_analytics_loggers::greenscale::{GreenscaleLogger, GreenscaleSettings};
use crate::http_analytics_loggers::tests::support::{
    COMPLETION, Capture, FixedClock, environment, event, field, json, time,
};

const START: &str = "2026-01-02T03:00:00Z";
const END: &str = "2026-01-02T03:00:01.250000Z";
const ENVIRONMENT: &[(&str, &str)] = &[
    ("GREENSCALE_API_KEY", "green-key"),
    (
        "GREENSCALE_ENDPOINT",
        "https://greenscale.example.com/v1/log",
    ),
];
const KWARGS: &str = r#"{"model":"gpt-4o","litellm_params":{"metadata":{}}}"#;

#[tokio::test]
async fn greenscale_stamps_the_utc_time_and_counts_tokens() {
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
    let body = &requests[0].body;
    assert_eq!(field(body, "timestamp"), &json(r#""2026-01-02T03:04:05Z""#));
    assert_eq!(field(body, "invocationLatency"), &json("1250"));
    assert_eq!(field(body, "inputTokenCount"), &json("3"));
    assert_eq!(field(body, "outputTokenCount"), &json("2"));
    assert_eq!(field(body, "modelId"), &json(r#""gpt-4o""#));
}
