use betterlite_openai::response_events::RawResponse;
use indexmap::IndexMap;
use serde_json::Value;

use crate::responses::decode::{cancel_reply, decode_azure_response_resource};
use crate::responses::error::ResponsesError;

fn with_reply<T>(status: u16, text: &str, run: impl FnOnce(&RawResponse<'_>) -> T) -> T {
    let headers = IndexMap::new();
    run(&RawResponse {
        status,
        headers: &headers,
        text,
    })
}

fn outcome(status: u16, text: &str) -> Value {
    with_reply(status, text, |reply| match cancel_reply(reply) {
        Ok(response) => serde_json::json!({
            "id": response.id,
            "created_at": response.created_at,
            "status": response.status,
            "model": response.model,
        }),
        Err(error) => {
            serde_json::json!({"status": error.http_status(), "message": error.to_string()})
        }
    })
}

pub(super) fn cancel_response() {
    let cancelled = outcome(
        200,
        r#"{"id":"resp_1","created_at":9,"output":[],"status":"cancelled","model":"gpt-5"}"#,
    );
    assert_eq!(
        cancelled,
        serde_json::json!({
            "id": "resp_1",
            "created_at": 9,
            "status": "cancelled",
            "model": "gpt-5"
        })
    );
    // a body that is not JSON fails as the upstream error, carrying its status
    let unparsed = outcome(502, "not json");
    assert_eq!(unparsed["status"], serde_json::json!(502));
    assert!(unparsed["message"].as_str().unwrap().contains("not json"));
    // a JSON body that is not a response record fails without an upstream status
    let invalid = outcome(200, r#"{"id":"resp_2","created_at":5,"output":"bad"}"#);
    assert_eq!(invalid["status"], Value::Null);
    // Azure parses a JSON array before the record check rejects it
    let array = outcome(200, "[]");
    assert_eq!(array["status"], Value::Null);
    decode_boundaries();
}

fn decode_boundaries() {
    let good = decode_azure_response_resource(
        br#"{"id":"resp_1","created_at":1,"output":[{"type":"message","id":"msg_1","role":"assistant","status":"completed","content":[]}]}"#,
    );
    assert_eq!(good.unwrap().id, "resp_1");
    assert!(matches!(
        decode_azure_response_resource(b"{\"id\":"),
        Err(ResponsesError::Record(_) | ResponsesError::Resource(_))
    ));
    assert!(matches!(
        decode_azure_response_resource(b""),
        Err(ResponsesError::Resource(_))
    ));
    assert!(matches!(
        decode_azure_response_resource(b"{\"id\":\"resp_1\",\"created_at\":1}"),
        Err(ResponsesError::Record(_))
    ));
}
