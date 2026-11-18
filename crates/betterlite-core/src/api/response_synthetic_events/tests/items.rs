#![cfg(test)]

use super::*;

#[test]
fn function_call_arguments_chunk_and_finish() {
    let output = r#"[{"type":"function_call","id":"fc_1","call_id":"call_1","name":"f","arguments":"{\"a\":1}","status":"completed"}]"#;
    let mut transformed = response(output);
    let items = build(&mut transformed, None, 5);
    assert_eq!(
        deltas(&items, "response.function_call_arguments.delta"),
        [r#"{"a":"#, "1}"]
    );
    let delta = event_of(&items, "response.function_call_arguments.delta");
    assert!(!has_key(delta, "content_index"));
    assert!(!has_key(delta, "sequence_number"));
    assert_eq!(
        text(field(
            event_of(&items, "response.function_call_arguments.done"),
            "arguments"
        )),
        r#"{"a":1}"#
    );
    let output = r#"[{"type":"function_call","id":"fc_1","call_id":"call_1","name":"f","arguments":"","status":"completed"}]"#;
    let mut transformed = response(output);
    let items = build(&mut transformed, None, 5);
    assert!(events_of(&items, "response.function_call_arguments.delta").is_empty());
    assert_eq!(
        text(field(
            event_of(&items, "response.function_call_arguments.done"),
            "arguments"
        )),
        ""
    );
}

#[test]
fn custom_tool_call_emits_only_item_events() {
    let output = concat!(
        r#"[{"type":"message","id":"msg_a","status":"completed","role":"assistant","content":[{"type":"output_text","text":"hi","annotations":[],"logprobs":null}]},"#,
        r#"{"type":"custom_tool_call","id":"ct_1","call_id":"call_1","name":"g","input":"raw","status":"completed"}]"#
    );
    let mut transformed = response(output);
    let items = build(&mut transformed, None, 5);
    assert!(
        !kinds(&items)
            .iter()
            .any(|event| event.contains("custom_tool_call"))
    );
    let added = events_of(&items, "response.output_item.added");
    assert_eq!(added.len(), 2);
    let custom = field(added[1], "item");
    assert_eq!(text(field(custom, "type")), "custom_tool_call");
    assert_eq!(text(field(custom, "input")), "raw");
    assert_eq!(text(field(custom, "status")), "completed");
    assert_eq!(field(custom, "caller"), &json("null"));
    assert_eq!(field(custom, "namespace"), &json("null"));
    let done = events_of(&items, "response.output_item.done");
    assert_eq!(field(done[1], "item"), custom);
    assert_eq!(integer(field(done[0], "sequence_number")), 1);
    assert_eq!(integer(field(done[1], "sequence_number")), 2);
    let completed = items.last().unwrap();
    let completed_custom = item_at(at(completed, &["response", "output"]), 1);
    assert_eq!(completed_custom, custom);
}

#[test]
fn reasoning_summaries_emit_done_events_with_sequence_numbers() {
    let output = concat!(
        r#"[{"type":"reasoning","id":"rs_1","summary":["#,
        r#"{"type":"summary_text","text":"one"},{"type":"summary_text","text":"two"}]}]"#
    );
    let mut transformed = response(output);
    let items = build(&mut transformed, None, 5);
    let deltas_with_index: Vec<(i64, String)> =
        events_of(&items, "response.reasoning_summary_text.delta")
            .iter()
            .map(|event| {
                (
                    integer(field(event, "summary_index")),
                    text(field(event, "delta")).to_owned(),
                )
            })
            .collect();
    assert_eq!(
        deltas_with_index,
        [(0, "one".to_owned()), (1, "two".to_owned())]
    );
    let delta = event_of(&items, "response.reasoning_summary_text.delta");
    assert!(!has_key(delta, "sequence_number"));
    let text_done = events_of(&items, "response.reasoning_summary_text.done");
    assert_eq!(text_done.len(), 2);
    assert_eq!(integer(field(text_done[0], "sequence_number")), 1);
    assert_eq!(integer(field(text_done[0], "summary_index")), 0);
    assert_eq!(text(field(text_done[0], "text")), "one");
    assert_eq!(integer(field(text_done[1], "sequence_number")), 3);
    assert_eq!(integer(field(text_done[1], "summary_index")), 1);
    let part_done = events_of(&items, "response.reasoning_summary_part.done");
    assert_eq!(integer(field(part_done[0], "sequence_number")), 2);
    assert_eq!(
        field(part_done[0], "part"),
        &json(r#"{"type":"summary_text","text":"one"}"#)
    );
    assert_eq!(integer(field(part_done[1], "sequence_number")), 4);
    let item_done = event_of(&items, "response.output_item.done");
    assert_eq!(integer(field(item_done, "sequence_number")), 5);
    assert_eq!(
        text(at(
            event_of(&items, "response.output_item.added"),
            &["item", "id"]
        )),
        "rs_1"
    );
}

#[test]
fn reasoning_summary_must_be_a_list() {
    let output = r#"[{"type":"reasoning","id":"rs_1","summary":"oops"}]"#;
    let mut transformed = response(output);
    let items = build(&mut transformed, None, 5);
    assert_eq!(
        kinds(&items),
        [
            "response.created",
            "response.in_progress",
            "response.output_item.added",
            "response.output_item.done",
            "response.completed",
        ]
    );
    assert_eq!(
        text(at(
            event_of(&items, "response.output_item.added"),
            &["item", "summary"]
        )),
        "oops"
    );
    assert_eq!(
        integer(field(
            event_of(&items, "response.output_item.done"),
            "sequence_number"
        )),
        1
    );
}

#[test]
fn completed_event_is_last_for_failed_and_incomplete_responses() {
    let extra = r#","error":{"code":"server_error"}"#;
    let mut transformed = parse::<ResponsesApiResponse>(&format!(
        r#"{{"id":"resp_a","created_at":1700000000,"status":"failed","output":[]{extra}}}"#
    ));
    let items = build(&mut transformed, None, 2);
    assert_eq!(
        kinds(&items),
        [
            "response.created",
            "response.in_progress",
            "response.completed"
        ]
    );
    let completed = items.last().unwrap();
    assert_eq!(text(at(completed, &["response", "status"])), "failed");
    assert_eq!(
        text(at(completed, &["response", "error", "code"])),
        "server_error"
    );
    let output = format!("[{}]", message_with_text("msg_a", "hi"));
    let mut transformed = parse::<ResponsesApiResponse>(&format!(
        r#"{{"id":"resp_a","created_at":1700000000,"status":"incomplete","output":{output}}}"#
    ));
    let items = build(&mut transformed, None, 1);
    assert_eq!(kind(items.last().unwrap()), "response.completed");
    assert_eq!(
        text(at(items.last().unwrap(), &["response", "status"])),
        "incomplete"
    );
}
