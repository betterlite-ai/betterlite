#![cfg(test)]

use super::{AdapterError, DispatchFailure, FixtureError};
use betterlite_env::EnvError;
use std::error::Error as _;

#[test]
fn dispatch_and_proxy_errors_keep_the_original_cause() {
    let fixture = FixtureError::Proxy(EnvError::Bool {
        name: String::from("AIOHTTP_TRUST_ENV"),
    });
    let dispatch = DispatchFailure::Fixture(fixture);
    let error = AdapterError::Dispatch(Box::new(dispatch));
    let cause = error.source().unwrap();
    let typed_source = cause.downcast_ref::<Box<DispatchFailure>>().unwrap();
    assert!(matches!(typed_source.as_ref(), DispatchFailure::Fixture(_)));
    let environment = cause.source().unwrap().downcast_ref::<EnvError>().unwrap();
    assert_eq!(
        environment,
        &EnvError::Bool {
            name: String::from("AIOHTTP_TRUST_ENV")
        }
    );
}

use super::super::{Input, Op};
use betterlite_oracle_helper_api::helper::Helper as _;
use serde_json::json;

#[test]
fn actual_http_completion_reaches_frames_and_history() {
    let input: Input = serde_json::from_value(json!({
        "setup": {"model":"openai/gpt-4o-mini", "litellm_metadata":null,
            "custom_llm_provider":null, "api_key":null, "api_base":null, "timeout":null,
            "first_message":null, "kwargs":{}},
        "frames":["{\"type\":\"response.create\",\"input\":\"hello\"}"],
        "disconnect":"closed", "quota":null, "sink":{"fails_at":null,"message":"send"},
        "scripts":[{"fail":null,"raise_after":null,"raise_message":"", "chunks":[{
            "kind":"json", "event_type":null, "dump":null,
            "value":{"type":"response.completed", "response":{"id":"wire-id", "output":[
                {"type":"message", "content":[{"type":"output_text", "text":"from HTTP"}]}]}}
        }]}]
    }))
    .unwrap();
    let answer = Op::run(&input).unwrap();
    assert_eq!(
        answer.pointer("/frames/0/response/output/0/content/0/text"),
        Some(&json!("from HTTP"))
    );
    assert_eq!(
        answer.pointer("/history/wire-id/1/content/0/text"),
        Some(&json!("from HTTP"))
    );
    assert_eq!(
        answer.pointer("/provider_requests/0/body/stream"),
        Some(&json!(true))
    );
    assert_ne!(
        answer.pointer("/frames/0/response/id"),
        Some(&json!("wire-id"))
    );
}
