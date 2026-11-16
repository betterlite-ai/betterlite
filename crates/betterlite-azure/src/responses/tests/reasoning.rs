use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::Json;

use super::support;
use crate::responses::reasoning::sanitize_item;
use crate::responses::request::transform_request;

fn transformed(items: &str) -> Vec<Json> {
    let input: ResponsesInput = serde_json::from_str(items).unwrap();
    let mut body =
        transform_request("azure/o_series/gpt-5", &input, &support::object("{}"), None).unwrap();
    let Some(Json::List(sent)) = body.shift_remove("input") else {
        panic!("the request input is not a list");
    };
    sent
}

fn list_len(item: &Json, key: &str) -> Option<usize> {
    match item.as_object()?.get(key)? {
        Json::List(entries) => Some(entries.len()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => None,
    }
}

fn summary_entries(count: u16) -> String {
    let entries: Vec<String> = (0..count)
        .map(|index| format!(r#"{{"type":"summary_text","text":"{index}"}}"#))
        .collect();
    entries.join(",")
}

pub(super) fn reasoning_null_fields() {
    let items = transformed(
        r#"[{
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
        {"role": "user", "content": "hi", "status": "completed"}]"#,
    );
    // a validated item drops its status even when the source carried one, and dumps its nulls
    let validated = r#"{
        "type": "reasoning",
        "id": "rs_1",
        "summary": [{"type": "summary_text", "text": "a", "y": 1}],
        "content": [{"type": "reasoning_text", "text": "t"}],
        "keep": {"nested": null}
    }"#;
    assert_eq!(
        items,
        [
            support::json(validated),
            // a message item loses every status field, a plain item keeps its own
            support::json(r#"{"type": "message", "role": "user", "content": "hi"}"#),
            support::json(r#"{"role": "user", "content": "hi", "status": "completed"}"#),
        ]
    );
    synthesized_summary();
    failed_validation();
    long_reasoning_content();
    sanitized_null_fields();
    sanitized_reasoning_content();
}

fn synthesized_summary() {
    let items = transformed(
        r#"[
        {"type": "reasoning", "id": "rs_2", "reasoning_content": [{"type": "summary_text", "text": "b"}]},
        {"type": "reasoning", "id": "rs_3", "summary": [{"type": "summary_text", "text": "a"}], "status": "incomplete"}
        ]"#,
    );
    // an absent summary is synthesized from the reasoning content list
    let synthesized = r#"{
        "type": "reasoning",
        "id": "rs_2",
        "summary": [{"type": "summary_text", "text": "b"}],
        "reasoning_content": [{"type": "summary_text", "text": "b"}]
    }"#;
    // validation removes any status, whichever literal it held
    let validated = r#"{"type": "reasoning", "id": "rs_3", "summary": [{"type": "summary_text", "text": "a"}]}"#;
    assert_eq!(
        items,
        [support::json(synthesized), support::json(validated)]
    );
}

fn failed_validation() {
    let items = transformed(
        r#"[
        {"type": "reasoning", "id": "rs_4", "summary": [{"type": "summary_text", "text": 5}], "status": "completed", "other": null},
        {"type": "reasoning", "id": "rs_5", "reasoning_content": "plain text", "status": "completed", "content": null},
        {"type": "reasoning", "id": "rs_6", "summary": [], "status": "bogus"}
        ]"#,
    );
    // a failed item keeps unrelated null fields, dropping only null status/content/encrypted_content
    let kept = r#"{"type": "reasoning", "id": "rs_4", "summary": [{"type": "summary_text", "text": 5}], "status": "completed", "other": null}"#;
    // text reasoning content cannot become a summary, so the source item survives
    let plain = r#"{"type": "reasoning", "id": "rs_5", "reasoning_content": "plain text", "status": "completed"}"#;
    let bogus = r#"{"type": "reasoning", "id": "rs_6", "summary": [], "status": "bogus"}"#;
    assert_eq!(
        items,
        [
            support::json(kept),
            support::json(plain),
            support::json(bogus),
        ]
    );
}

fn long_reasoning_content() {
    let at_limit = summary_entries(100);
    let over_limit = summary_entries(101);
    let items = transformed(&format!(
        r#"[
        {{"type": "reasoning", "id": "rs_100", "reasoning_content": [{at_limit}], "status": "completed"}},
        {{"type": "reasoning", "id": "rs_101", "reasoning_content": [{over_limit}]}}
        ]"#
    ));
    // exactly one hundred entries still synthesize a summary, and the item loses the status it carried
    assert_eq!(list_len(&items[0], "summary"), Some(100_usize));
    let fields = items[0].as_object().unwrap();
    assert!(!fields.contains_key("status"));
    // one more entry makes LiteLLM's slice-plus-ellipsis raise, so the item is kept as it arrived
    assert_eq!(list_len(&items[1], "summary"), None);
    assert_eq!(list_len(&items[1], "reasoning_content"), Some(101_usize));
}

fn sanitized_null_fields() {
    // The Azure filter alone, since the later shared request pass would drop these nulls too
    let filtered = Json::Object(sanitize_item(&support::object(
        r#"{"type": "reasoning", "id": "rs_7", "summary": [{"type": "summary_text", "text": "a", "extra": null, "y": 1}],
            "status": "completed", "encrypted_content": null, "znull": null, "keep": {"nested": null}}"#,
    )));
    assert_eq!(
        filtered,
        support::json(
            r#"{"type": "reasoning", "id": "rs_7", "summary": [{"type": "summary_text", "text": "a", "y": 1}], "keep": {"nested": null}}"#
        )
    );
}

fn sanitized_reasoning_content() {
    let at_limit_entries = summary_entries(100);
    let at_limit = Json::Object(sanitize_item(&support::object(&format!(
        r#"{{"type": "reasoning", "id": "rs_100", "reasoning_content": [{at_limit_entries}], "status": "completed"}}"#
    ))));
    // exactly one hundred entries validate, so the filter drops the status it carried
    assert_eq!(list_len(&at_limit, "summary"), Some(100_usize));
    let fields = at_limit.as_object().unwrap();
    assert!(!fields.contains_key("status"));
    let over_limit_entries = summary_entries(101);
    let over_limit = Json::Object(sanitize_item(&support::object(&format!(
        r#"{{"type": "reasoning", "id": "rs_101", "reasoning_content": [{over_limit_entries}]}}"#
    ))));
    // one more entry fails validation, so the source item survives with no synthesized summary
    assert_eq!(list_len(&over_limit, "summary"), None);
    assert_eq!(list_len(&over_limit, "reasoning_content"), Some(101_usize));
}
