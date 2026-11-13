use crate::http_analytics_loggers::promptlayer::{PromptLayerLogger, PromptLayerSettings};
use crate::http_analytics_loggers::tests::support::{
    Answer, COMPLETION, Capture, environment, event, field, json,
};

const START: &str = "2026-01-02T03:00:00Z";
const END: &str = "2026-01-02T03:00:01Z";
const ENVIRONMENT: &[(&str, &str)] = &[("PROMPTLAYER_API_KEY", "pl-key")];
const KWARGS: &str = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"Hello"}],"optional_params":{},"litellm_params":{"metadata":{"pl_tags":["prod"],"user":"u1","team":"t1"}}}"#;
const TAGGED: &str = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"Hello"}],"optional_params":{},"litellm_params":{"metadata":{"pl_tags":["prod"]}}}"#;

#[tokio::test]
async fn promptlayer_posts_the_remaining_metadata_after_a_confirmed_request() {
    let capture = Capture::with_answers([
        Answer::Ok(json(r#"{"success":true,"request_id":"req-1"}"#)),
        Answer::Ok(json("{}")),
    ]);
    let logger = PromptLayerLogger {
        delivery: capture.clone(),
        settings: PromptLayerSettings::from_environment(&environment(ENVIRONMENT)),
    };
    let response = json(COMPLETION);
    logger
        .log_event(&event(&json(KWARGS), Some(&response), START, END))
        .await;
    let requests = capture.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].url,
        "https://api.promptlayer.com/rest/track-metadata"
    );
    let body = &requests[1].body;
    assert_eq!(field(body, "request_id"), &json(r#""req-1""#));
    assert_eq!(field(body, "api_key"), &json(r#""pl-key""#));
    assert_eq!(
        field(body, "metadata"),
        &json(r#"{"user":"u1","team":"t1"}"#)
    );
}

#[tokio::test]
async fn promptlayer_skips_the_metadata_post_when_only_tags_remain() {
    let capture =
        Capture::with_answers([Answer::Ok(json(r#"{"success":true,"request_id":"req-1"}"#))]);
    let logger = PromptLayerLogger {
        delivery: capture.clone(),
        settings: PromptLayerSettings::from_environment(&environment(ENVIRONMENT)),
    };
    let response = json(COMPLETION);
    logger
        .log_event(&event(&json(TAGGED), Some(&response), START, END))
        .await;
    assert_eq!(capture.requests().len(), 1);
}

#[tokio::test]
async fn promptlayer_skips_the_metadata_post_without_a_request_id() {
    let capture = Capture::with_answers([Answer::Ok(json(r#"{"success":true}"#))]);
    let logger = PromptLayerLogger {
        delivery: capture.clone(),
        settings: PromptLayerSettings::from_environment(&environment(ENVIRONMENT)),
    };
    let response = json(COMPLETION);
    logger
        .log_event(&event(&json(KWARGS), Some(&response), START, END))
        .await;
    assert_eq!(capture.requests().len(), 1);
}

#[tokio::test]
async fn promptlayer_stops_when_the_service_does_not_confirm_the_request() {
    let capture = Capture::with_answers([Answer::Ok(json(r#"{"success":false}"#))]);
    let logger = PromptLayerLogger {
        delivery: capture.clone(),
        settings: PromptLayerSettings::from_environment(&environment(ENVIRONMENT)),
    };
    let response = json(COMPLETION);
    logger
        .log_event(&event(&json(KWARGS), Some(&response), START, END))
        .await;
    assert_eq!(capture.requests().len(), 1);
}
