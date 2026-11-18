#![cfg(test)]

use super::*;

#[test]
fn refusal_parts_emit_delta_and_done() {
    let output = r#"[{"type":"message","id":"msg_a","status":"completed","role":"assistant","content":[{"type":"refusal","refusal":"nope"}]}]"#;
    let mut transformed = response(output);
    let items = build(&mut transformed, None, 2);
    assert_eq!(
        kinds(&items),
        [
            "response.created",
            "response.in_progress",
            "response.output_item.added",
            "response.content_part.added",
            "response.refusal.delta",
            "response.refusal.delta",
            "response.refusal.done",
            "response.content_part.done",
            "response.output_item.done",
            "response.completed",
        ]
    );
    assert_eq!(deltas(&items, "response.refusal.delta"), ["no", "pe"]);
    assert_eq!(
        text(field(event_of(&items, "response.refusal.done"), "refusal")),
        "nope"
    );
    assert_eq!(
        field(event_of(&items, "response.content_part.done"), "part"),
        &json(r#"{"type":"refusal","refusal":"nope"}"#)
    );
    assert!(events_of(&items, "response.output_text.delta").is_empty());
}

#[test]
fn reasoning_text_parts_emit_only_part_done() {
    let output = r#"[{"type":"message","id":"msg_a","status":"completed","role":"assistant","content":[{"type":"reasoning_text","reasoning":"why"}]}]"#;
    let mut transformed = response(output);
    let items = build(&mut transformed, None, 2);
    assert_eq!(
        kinds(&items),
        [
            "response.created",
            "response.in_progress",
            "response.output_item.added",
            "response.content_part.added",
            "response.content_part.done",
            "response.output_item.done",
            "response.completed",
        ]
    );
    assert_eq!(
        field(event_of(&items, "response.content_part.done"), "part"),
        &json(r#"{"type":"reasoning_text","reasoning":"why"}"#)
    );
}

#[test]
fn unknown_part_types_emit_only_part_added() {
    let output = r#"[{"type":"message","id":"msg_a","status":"completed","role":"assistant","content":[{"type":"custom_part","x":1}]}]"#;
    let mut transformed = response(output);
    let items = build(&mut transformed, None, 2);
    assert_eq!(
        kinds(&items),
        [
            "response.created",
            "response.in_progress",
            "response.output_item.added",
            "response.content_part.added",
            "response.output_item.done",
            "response.completed",
        ]
    );
    assert_eq!(
        field(event_of(&items, "response.content_part.added"), "part"),
        &json(r#"{"type":"custom_part","x":1}"#)
    );
}

#[test]
fn non_list_content_emits_no_part_events() {
    let output = r#"[{"type":"message","id":"msg_a","status":"completed","role":"assistant","content":"oops"}]"#;
    let mut transformed = response(output);
    let items = build(&mut transformed, None, 2);
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
    let item_added = event_of(&items, "response.output_item.added");
    assert_eq!(text(at(item_added, &["item", "content"])), "oops");
}

#[test]
fn falsy_text_fields_become_empty_strings() {
    let output = concat!(
        r#"[{"type":"message","id":"msg_a","status":"completed","role":"assistant","content":["#,
        r#"{"type":"output_text","text":0},"#,
        r#"{"type":"output_text","text":false},"#,
        r#"{"type":"output_text","text":null},"#,
        r#"{"type":"refusal","refusal":0},"#,
        r#"{"type":"reasoning_text","reasoning":""},"#,
        r#"{"type":"output_text","text":"x","annotations":0,"logprobs":null}]}]"#
    );
    let mut transformed = response(output);
    let items = build(&mut transformed, None, 5);
    assert_eq!(deltas(&items, "response.output_text.delta"), ["x"]);
    let done_texts: Vec<String> = events_of(&items, "response.output_text.done")
        .iter()
        .map(|event| text(field(event, "text")).to_owned())
        .collect();
    assert_eq!(done_texts, ["", "", "", "x"]);
    assert_eq!(
        text(field(event_of(&items, "response.refusal.done"), "refusal")),
        ""
    );
    let reasoning_done = events_of(&items, "response.content_part.done")
        .into_iter()
        .find(|event| text(at(event, &["part", "type"])) == "reasoning_text")
        .unwrap();
    assert_eq!(text(at(reasoning_done, &["part", "reasoning"])), "");
    let dones = events_of(&items, "response.content_part.done");
    let last_done = dones.last().copied().unwrap();
    assert_eq!(at(last_done, &["part", "annotations"]), &json("[]"));
}

#[test]
fn truthy_text_fields_stringify_python_style() {
    let output = concat!(
        r#"[{"type":"message","id":"msg_a","status":"completed","role":"assistant","content":["#,
        r#"{"type":"output_text","text":123},"#,
        r#"{"type":"output_text","text":true},"#,
        r#"{"type":"output_text","text":[1,2]},"#,
        r#"{"type":"output_text","text":{"a":1}},"#,
        r#"{"type":"refusal","refusal":7},"#,
        r#"{"type":"reasoning_text","reasoning":9}]}]"#
    );
    let mut transformed = response(output);
    let items = build(&mut transformed, None, 10);
    assert_eq!(
        deltas(&items, "response.output_text.delta"),
        ["123", "True", "[1, 2]", "{'a': 1}"]
    );
    assert_eq!(deltas(&items, "response.refusal.delta"), ["7"]);
    let reasoning_done = events_of(&items, "response.content_part.done")
        .into_iter()
        .find(|event| text(at(event, &["part", "type"])) == "reasoning_text")
        .unwrap();
    assert_eq!(text(at(reasoning_done, &["part", "reasoning"])), "9");
}

#[test]
fn unsupported_objects_project_to_empty_payloads() {
    let output = r#"[{"type":"message","id":"msg_a","status":"completed","role":"assistant","content":[7,"x"]}]"#;
    let mut transformed = response(output);
    let items = build(&mut transformed, None, 2);
    let added = events_of(&items, "response.content_part.added");
    assert_eq!(added.len(), 2);
    assert_eq!(field(added[0], "part"), &json("{}"));
    assert_eq!(field(added[1], "part"), &json("{}"));
    assert!(events_of(&items, "response.content_part.done").is_empty());
    assert!(events_of(&items, "response.output_text.delta").is_empty());
}

#[test]
fn missing_or_falsy_item_id_falls_back_to_response_id() {
    let cases = [
        (
            r#"{"type":"message","id":null,"status":"completed","role":"assistant","content":[{"type":"output_text","text":"hi","annotations":[],"logprobs":null}]}"#,
            "resp_a",
            true,
        ),
        (
            r#"{"type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"hi","annotations":[],"logprobs":null}]}"#,
            "resp_a",
            false,
        ),
        (
            r#"{"type":"message","id":5,"status":"completed","role":"assistant","content":[{"type":"output_text","text":"hi","annotations":[],"logprobs":null}]}"#,
            "5",
            true,
        ),
        (
            r#"{"type":"message","id":0,"status":"completed","role":"assistant","content":[{"type":"output_text","text":"hi","annotations":[],"logprobs":null}]}"#,
            "resp_a",
            true,
        ),
    ];
    for (item, expected_id, has_id) in cases {
        let output = format!("[{item}]");
        let mut transformed = response(&output);
        let items = build(&mut transformed, None, 5);
        let part_added = event_of(&items, "response.content_part.added");
        assert_eq!(text(field(part_added, "item_id")), expected_id);
        let delta = event_of(&items, "response.output_text.delta");
        assert_eq!(text(field(delta, "item_id")), expected_id);
        let item_added = event_of(&items, "response.output_item.added");
        assert_eq!(has_key(field(item_added, "item"), "id"), has_id);
        let item_done = event_of(&items, "response.output_item.done");
        assert_eq!(has_key(field(item_done, "item"), "id"), has_id);
    }
}

#[test]
fn indices_preserve_source_positions() {
    let output = concat!(
        r#"[{"type":"function_call","id":"fc_1","call_id":"call_1","name":"f","arguments":"{}","status":"completed"},"#,
        r#"{"type":"message","id":"msg_a","status":"completed","role":"assistant","content":["#,
        r#"{"type":"custom_part"},"#,
        r#"{"type":"output_text","text":"x","annotations":[],"logprobs":null}]}]"#
    );
    let mut transformed = response(output);
    let items = build(&mut transformed, None, 5);
    let fc_done = events_of(&items, "response.function_call_arguments.done");
    assert_eq!(integer(field(fc_done[0], "output_index")), 0);
    let parts = events_of(&items, "response.content_part.added");
    assert_eq!(integer(field(parts[0], "output_index")), 1);
    assert_eq!(integer(field(parts[0], "content_index")), 0);
    assert_eq!(integer(field(parts[1], "content_index")), 1);
    let delta = event_of(&items, "response.output_text.delta");
    assert_eq!(integer(field(delta, "output_index")), 1);
    assert_eq!(integer(field(delta, "content_index")), 1);
}
