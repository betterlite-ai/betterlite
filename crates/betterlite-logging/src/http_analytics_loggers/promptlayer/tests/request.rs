use crate::http_analytics_loggers::promptlayer::{PromptLayerLogger, PromptLayerSettings};
use crate::http_analytics_loggers::tests::support::{
    Answer, COMPLETION, Capture, environment, event, field, json, object,
};

const START: &str = "2026-01-02T03:00:00Z";
const END: &str = "2026-01-02T03:00:01.250000Z";
const ENVIRONMENT: &[(&str, &str)] = &[("PROMPTLAYER_API_KEY", "pl-key")];
const KWARGS: &str = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"Hello"}],"optional_params":{"temperature":0.2,"top_p":1},"litellm_params":{"metadata":{"pl_tags":["prod","chat"],"user":"u1","team":"t1"}}}"#;

#[tokio::test]
async fn promptlayer_posts_the_track_request_envelope() {
    let capture = Capture::default();
    let logger = PromptLayerLogger {
        delivery: capture.clone(),
        settings: PromptLayerSettings::from_environment(&environment(ENVIRONMENT)),
    };
    let response = json(COMPLETION);
    logger
        .log_event(&event(&json(KWARGS), Some(&response), START, END))
        .await;
    let requests = capture.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].url,
        "https://api.promptlayer.com/rest/track-request"
    );
    assert_eq!(
        requests[0].headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    let body = &requests[0].body;
    assert_eq!(
        field(body, "function_name"),
        &json(r#""openai.ChatCompletion.create""#)
    );
    assert_eq!(
        field(body, "kwargs"),
        &json(
            r#"{"model":"gpt-4o","messages":[{"role":"user","content":"Hello"}],"temperature":0.2,"top_p":1}"#
        )
    );
    assert_eq!(field(body, "request_start_time"), &json("1767322800"));
    assert_eq!(field(body, "request_end_time"), &json("1767322801"));
    assert_eq!(field(body, "api_key"), &json(r#""pl-key""#));
    assert!(!object(body).contains_key("metadata"));
}

#[tokio::test]
async fn promptlayer_posts_a_null_api_key_without_the_variable() {
    let capture = Capture::with_answers([Answer::Ok(json(r#"{"success":true}"#))]);
    let logger = PromptLayerLogger {
        delivery: capture.clone(),
        settings: PromptLayerSettings::from_environment(&environment(&[])),
    };
    let response = json(COMPLETION);
    logger
        .log_event(&event(&json(KWARGS), Some(&response), START, END))
        .await;
    let requests = capture.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(field(&requests[0].body, "api_key"), &json("null"));
}
