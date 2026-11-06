use crate::http_analytics_loggers::athina::{AthinaLogger, AthinaSettings};
use crate::http_analytics_loggers::tests::support::{
    COMPLETION, Capture, environment, event, field, json, object,
};

const START: &str = "2026-01-02T03:00:00Z";
const END: &str = "2026-01-02T03:00:01.250000Z";
const KEY: &[(&str, &str)] = &[("ATHINA_API_KEY", "athina-key-1")];
const KWARGS: &str = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"Hello"}],"optional_params":{"temperature":0.2,"tools":[{"type":"function","function":{"name":"get_weather"}}],"functions":[{"name":"legacy"}]},"litellm_params":{"metadata":{"environment":"prod","prompt_slug":"welcome","customer_id":"cust-1","customer_user_id":"user-1","session_id":"session-1","external_reference_id":"ext-1","context":{"page":1},"expected_response":"expected","user_query":"query","tags":["a"],"user_feedback":"good","model_options":{"temperature":0.2},"custom_attributes":{"tier":"gold"},"unknown_key":"dropped"}}}"#;

#[tokio::test]
async fn athina_builds_the_payload_from_kwargs_and_the_response() {
    let capture = Capture::default();
    let logger = AthinaLogger {
        delivery: capture.clone(),
        settings: AthinaSettings::from_environment(&environment(KEY)),
    };
    let kwargs = json(KWARGS);
    let response = json(COMPLETION);
    logger
        .log_event(&event(&kwargs, Some(&response), START, END))
        .await;
    let requests = capture.requests();
    assert_eq!(requests.len(), 1);
    let body = &requests[0].body;
    assert_eq!(field(body, "language_model_id"), &json(r#""gpt-4o""#));
    assert_eq!(field(body, "request"), &kwargs);
    assert_eq!(field(body, "response"), &response);
    assert_eq!(field(body, "prompt_tokens"), &json("3"));
    assert_eq!(field(body, "completion_tokens"), &json("2"));
    assert_eq!(field(body, "total_tokens"), &json("5"));
    assert_eq!(field(body, "response_time"), &json("1250"));
    assert_eq!(field(body, "prompt"), field(&kwargs, "messages"));
    assert_eq!(
        field(body, "tools"),
        field(field(&kwargs, "optional_params"), "tools")
    );
    assert_eq!(
        field(body, "functions"),
        field(field(&kwargs, "optional_params"), "functions")
    );
    assert!(!object(body).contains_key("unknown_key"));
    assert!(!object(body).contains_key("temperature"));
}

#[tokio::test]
async fn athina_copies_every_additional_metadata_key() {
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
    let body = &requests[0].body;
    assert_eq!(field(body, "environment"), &json(r#""prod""#));
    assert_eq!(field(body, "prompt_slug"), &json(r#""welcome""#));
    assert_eq!(field(body, "customer_id"), &json(r#""cust-1""#));
    assert_eq!(field(body, "customer_user_id"), &json(r#""user-1""#));
    assert_eq!(field(body, "session_id"), &json(r#""session-1""#));
    assert_eq!(field(body, "external_reference_id"), &json(r#""ext-1""#));
    assert_eq!(field(body, "context"), &json(r#"{"page":1}"#));
    assert_eq!(field(body, "expected_response"), &json(r#""expected""#));
    assert_eq!(field(body, "user_query"), &json(r#""query""#));
    assert_eq!(field(body, "tags"), &json(r#"["a"]"#));
    assert_eq!(field(body, "user_feedback"), &json(r#""good""#));
    assert_eq!(
        field(body, "model_options"),
        &json(r#"{"temperature":0.2}"#)
    );
    assert_eq!(
        field(body, "custom_attributes"),
        &json(r#"{"tier":"gold"}"#)
    );
}

#[tokio::test]
async fn athina_omits_the_prompt_and_extra_keys_without_messages_and_metadata() {
    let capture = Capture::default();
    let logger = AthinaLogger {
        delivery: capture.clone(),
        settings: AthinaSettings::from_environment(&environment(KEY)),
    };
    let kwargs = json(r#"{"model":"gpt-4o","litellm_params":{"metadata":{}}}"#);
    let response = json(COMPLETION);
    logger
        .log_event(&event(&kwargs, Some(&response), START, END))
        .await;
    let requests = capture.requests();
    let body = &requests[0].body;
    assert!(!object(body).contains_key("prompt"));
    assert!(!object(body).contains_key("tools"));
    assert_eq!(field(body, "response_time"), &json("1250"));
}
