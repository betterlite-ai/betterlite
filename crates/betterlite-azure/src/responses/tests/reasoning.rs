use betterlite_openai::response_input::ResponsesInput;
use serde_json::{Value, json};

use super::support;
use crate::responses::request::transform_request;

fn transformed(items: Value) -> Vec<Value> {
    let input: ResponsesInput = serde_json::from_value(items).unwrap();
    let body =
        transform_request("azure/o_series/gpt-5", &input, &support::object("{}"), None).unwrap();
    serde_json::to_value(body).unwrap()["input"]
        .as_array()
        .unwrap()
        .clone()
}

pub(super) fn reasoning_null_fields() {
    let items = transformed(json!([
        {
            "type": "reasoning",
            "id": "rs_1",
            "summary": [{"type": "summary_text", "text": "a", "extra": null, "y": 1}],
            "status": "completed",
            "encrypted_content": null,
            "content": [{"type": "reasoning_text", "text": "t"}],
            "znull": null,
            "keep": {"nested": null}
        },
        {"type": "message", "role": "user", "content": "hi", "status": "completed"},
        {"role": "user", "content": "hi", "status": "completed"},
        {"type": "reasoning", "id": "rs_2", "reasoning_content": [{"type": "summary_text", "text": "b"}]},
        {
            "type": "reasoning",
            "id": "rs_3",
            "summary": [{"type": "summary_text", "text": "a"}],
            "status": "incomplete"
        },
        {
            "type": "reasoning",
            "id": "rs_4",
            "summary": [{"type": "summary_text", "text": 5}],
            "status": "completed",
            "other": null
        },
        {"type": "reasoning", "id": "rs_5", "reasoning_content": "plain text", "status": "completed", "content": null},
        {"type": "reasoning", "id": "rs_6", "summary": [], "status": "bogus"}
    ]));
    // a validated item drops its status even when the source carried one, and dumps its nulls
    assert_eq!(
        items[0],
        json!({
            "type": "reasoning",
            "id": "rs_1",
            "summary": [{"type": "summary_text", "text": "a", "y": 1}],
            "content": [{"type": "reasoning_text", "text": "t"}],
            "keep": {"nested": null}
        })
    );
    // a message item loses every status field, a plain item keeps its own
    assert_eq!(
        items[1],
        json!({"type": "message", "role": "user", "content": "hi"})
    );
    assert_eq!(
        items[2],
        json!({"role": "user", "content": "hi", "status": "completed"})
    );
    // an absent summary is synthesized from the reasoning content list
    assert_eq!(
        items[3],
        json!({
            "type": "reasoning",
            "id": "rs_2",
            "summary": [{"type": "summary_text", "text": "b"}],
            "reasoning_content": [{"type": "summary_text", "text": "b"}]
        })
    );
    // validation removes any status, whichever literal it held
    assert_eq!(
        items[4],
        json!({
            "type": "reasoning",
            "id": "rs_3",
            "summary": [{"type": "summary_text", "text": "a"}]
        })
    );
    // and keeps unrelated null fields, dropping only null status/content/encrypted_content
    assert_eq!(
        items[5],
        json!({
            "type": "reasoning",
            "id": "rs_4",
            "summary": [{"type": "summary_text", "text": 5}],
            "status": "completed",
            "other": null
        })
    );
    // text reasoning content cannot become a summary, so the source item survives
    assert_eq!(
        items[6],
        json!({"type": "reasoning", "id": "rs_5", "reasoning_content": "plain text", "status": "completed"})
    );
    assert_eq!(
        items[7],
        json!({"type": "reasoning", "id": "rs_6", "summary": [], "status": "bogus"})
    );
    long_reasoning_content();
}

fn long_reasoning_content() {
    let at_limit: Vec<Value> = (0..100)
        .map(|index| json!({"type": "summary_text", "text": index.to_string()}))
        .collect();
    let over_limit: Vec<Value> = (0..101)
        .map(|index| json!({"type": "summary_text", "text": index.to_string()}))
        .collect();
    let items = transformed(json!([
        {"type": "reasoning", "id": "rs_100", "reasoning_content": at_limit},
        {"type": "reasoning", "id": "rs_101", "reasoning_content": over_limit}
    ]));
    // exactly one hundred entries still synthesize a summary
    assert_eq!(
        items[0]["summary"].as_array().map(Vec::len),
        Some(100_usize)
    );
    // one more entry makes LiteLLM's slice-plus-ellipsis raise, so the item is kept as it arrived
    assert_eq!(items[1].get("summary"), None);
    assert_eq!(
        items[1]["reasoning_content"].as_array().map(Vec::len),
        Some(101_usize)
    );
}
