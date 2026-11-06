use crate::http_analytics_loggers::greenscale::{GreenscaleLogger, GreenscaleSettings};
use crate::http_analytics_loggers::tests::support::{
    COMPLETION, Capture, FixedClock, environment, event, field, json, object, time,
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
const KWARGS: &str = r#"{"model":"gpt-4o","litellm_params":{"metadata":{"greenscale_project":"proj","greenscale_application":"app","greenscale_attempt":2,"greenscale_greenscale_extra":"nested","other":"x"}}}"#;

#[tokio::test]
async fn greenscale_splits_project_application_and_tag_metadata() {
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
    let body = &requests[0].body;
    assert_eq!(field(body, "project"), &json(r#""proj""#));
    assert_eq!(field(body, "application"), &json(r#""app""#));
    assert_eq!(
        field(body, "tags"),
        &json(r#"[{"key":"attempt","value":"2"},{"key":"extra","value":"nested"}]"#)
    );
}

#[tokio::test]
async fn greenscale_ignores_metadata_without_the_greenscale_prefix() {
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
    let kwargs = json(
        r#"{"model":"gpt-4o","litellm_params":{"metadata":{"other":"x","greenscale_project":"proj"}}}"#,
    );
    logger
        .log_event(&event(&kwargs, Some(&response), START, END))
        .await;
    let requests = capture.requests();
    let body = &requests[0].body;
    assert_eq!(field(body, "project"), &json(r#""proj""#));
    assert_eq!(field(body, "tags"), &json("[]"));
    assert!(!object(body).contains_key("other"));
}
