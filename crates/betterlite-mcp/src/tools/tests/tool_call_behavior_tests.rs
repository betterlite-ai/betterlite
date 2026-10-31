#![cfg(test)]

use super::{assert_json, fake, timeouts};
use crate::tools::pagination::ToolError;
use crate::tools::tool_calls::{
    CallRequest, OpenAiToolCall, call_mcp_tool, call_openai_tool, openai_tool_call_to_mcp,
};
use rstest::rstest;

use std::time::Duration;

#[rstest]
#[case::json(r#""{\"x\":1}""#, r#"{"x":1}"#)]
#[case::dict(r#"{"x":1}"#, r#"{"x":1}"#)]
#[case::invalid(r#""broken""#, "{}")]
#[case::empty(r#""""#, "{}")]
#[case::array("[]", "{}")]
#[case::json_array(r#""[]""#, "{}")]
#[case::number("3", "{}")]
#[case::boolean("true", "{}")]
#[case::null("null", "{}")]
#[tokio::test(start_paused = true)]
async fn parses_and_invokes(#[case] arguments: &str, #[case] expected: &str) {
    let call: OpenAiToolCall = serde_json::from_str(&format!(
        r#"{{"function":{{"name":"echo","arguments":{arguments}}}}}"#
    ))
    .unwrap();
    let request: CallRequest =
        serde_json::from_str(&format!(r#"{{"name":"echo","arguments":{expected}}}"#)).unwrap();
    assert_eq!(openai_tool_call_to_mcp(&call), request);
    let mut session = fake(Vec::new());
    assert_eq!(
        call_openai_tool(&mut session, &call, timeouts(60, 30))
            .await
            .unwrap(),
        request
    );
    assert_eq!(session.calls, vec![request]);
}

#[tokio::test(start_paused = true)]
async fn missing_args_and_direct_calls() {
    let call: OpenAiToolCall = serde_json::from_str(r#"{"function":{"name":"echo"}}"#).unwrap();
    assert_json(
        &openai_tool_call_to_mcp(&call),
        r#"{"name":"echo","arguments":{}}"#,
    );
    let request = CallRequest {
        name: "raw".to_owned(),
        arguments: None,
    };
    let mut session = fake(Vec::new());
    assert_eq!(
        call_mcp_tool(&mut session, request.clone(), timeouts(60, 30))
            .await
            .unwrap(),
        request
    );
    assert_eq!(session.calls, vec![request]);
}

#[tokio::test(start_paused = true)]
async fn calls_obey_the_client_timeout() {
    let call: OpenAiToolCall = serde_json::from_str(r#"{"function":{"name":"echo"}}"#).unwrap();
    let mut session = fake(Vec::new());
    session.call_delay = Duration::from_secs(40);
    let response = call_openai_tool(&mut session, &call, timeouts(60, 30))
        .await
        .unwrap();
    assert_eq!(response.name, "echo");
    assert!(matches!(
        call_openai_tool(&mut session, &call, timeouts(30, 60)).await,
        Err(ToolError::Timeout(_))
    ));
}
