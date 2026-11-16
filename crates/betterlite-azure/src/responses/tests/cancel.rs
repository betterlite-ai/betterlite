use betterlite_openai::response_events::RawResponse;
use indexmap::IndexMap;

use crate::responses::decode::{cancel_reply, decode_azure_response_resource};
use crate::responses::error::ResponsesError;

fn with_reply<T, R>(status: u16, text: &str, run: R) -> T
where
    R: FnOnce(&RawResponse<'_>) -> T,
{
    let headers = IndexMap::new();
    run(&RawResponse {
        status,
        headers: &headers,
        text,
    })
}

pub(super) fn cancel_response() {
    let body = r#"{"id":"resp_1","created_at":9,"output":[],"status":"cancelled","model":"gpt-5"}"#;
    let cancelled = with_reply(200, body, cancel_reply).unwrap();
    assert_eq!(
        (
            cancelled.id.as_str(),
            cancelled.created_at,
            cancelled.status.as_deref(),
            cancelled.model.as_deref(),
        ),
        ("resp_1", 9_i64, Some("cancelled"), Some("gpt-5"))
    );
    // a body that is not JSON fails as the upstream error, carrying the reply's own status
    let unparsed = with_reply(503, "still not json", cancel_reply).unwrap_err();
    assert_eq!(unparsed.http_status(), Some(503_u16));
    assert!(unparsed.to_string().contains("still not json"));
    // a JSON body that is not a response record fails without an upstream status
    let invalid = r#"{"id":"resp_2","created_at":5,"output":"bad"}"#;
    let record = with_reply(200, invalid, cancel_reply).unwrap_err();
    assert_eq!(record.http_status(), None);
    // Azure parses a JSON array before the record check rejects it
    let array = with_reply(200, "[]", cancel_reply).unwrap_err();
    assert_eq!(array.http_status(), None);
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
