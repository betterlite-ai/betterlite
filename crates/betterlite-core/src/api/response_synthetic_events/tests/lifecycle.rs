#![cfg(test)]

use super::*;

fn assert_in_progress_event(event: &Json) {
    let response = field(event, "response");
    assert_eq!(text(field(response, "status")), "in_progress");
    assert_eq!(field(response, "output"), &json("[]"));
    assert_eq!(text(field(field(response, "error"), "code")), "previous");
    assert_eq!(text(field(response, "instructions")), "be nice");
    assert_eq!(text(field(response, "model")), "gpt-4o");
    assert_eq!(text(field(field(response, "metadata"), "trace")), "t1");
    assert_eq!(number_text(at(response, &["usage", "cost"])), "0.5");
    assert_eq!(text(field(response, "id")), "resp_a");
    assert_eq!(integer(field(response, "created_at")), 1_700_000_000);
}

#[test]
fn synthetic_text_sequence_matches_litellm() {
    let output = concat!(
        r#"[{"type":"message","id":"msg_a","status":"completed","role":"assistant","content":["#,
        r#"{"type":"output_text","text":"abcdef","annotations":["#,
        r#"{"type":"url_citation","url":"https://example.test/doc","title":"Doc","start_index":0,"end_index":1}"#,
        r#"],"logprobs":null}]}]"#
    );
    let usage_json = r#","usage":{"input_tokens":2,"output_tokens":3,"total_tokens":5,"cost":0.5}"#;
    let mut transformed = response_with(output, usage_json);
    let original = transformed.clone();
    let items = build(&mut transformed, None, 5);
    assert_eq!(
        kinds(&items),
        [
            "response.created",
            "response.in_progress",
            "response.output_item.added",
            "response.content_part.added",
            "response.output_text.delta",
            "response.output_text.delta",
            "response.output_text.annotation.added",
            "response.output_text.done",
            "response.content_part.done",
            "response.output_item.done",
            "response.completed",
        ]
    );
    assert_eq!(deltas(&items, "response.output_text.delta"), ["abcde", "f"]);
    assert_eq!(
        deltas(&items, "response.output_text.delta")
            .into_iter()
            .collect::<String>(),
        "abcdef"
    );
    let created = event_of(&items, "response.created");
    assert_eq!(text(at(created, &["response", "status"])), "in_progress");
    assert_eq!(at(created, &["response", "output"]), &json("[]"));
    assert_eq!(
        number_text(at(created, &["response", "usage", "cost"])),
        "0.5"
    );
    let part_added = event_of(&items, "response.content_part.added");
    assert_eq!(text(at(part_added, &["part", "text"])), "abcdef");
    let annotation = event_of(&items, "response.output_text.annotation.added");
    assert_eq!(text(at(annotation, &["annotation", "title"])), "Doc");
    assert_eq!(integer(field(annotation, "annotation_index")), 0);
    let text_done = event_of(&items, "response.output_text.done");
    assert_eq!(text(field(text_done, "text")), "abcdef");
    let part_done = event_of(&items, "response.content_part.done");
    assert_eq!(text(at(part_done, &["part", "text"])), "abcdef");
    assert_eq!(
        at(part_done, &["part", "annotations"]),
        &json(&format!(
            r#"[{{"type":"url_citation","url":"https://example.test/doc","title":"Doc","start_index":0,"end_index":1}}]"#
        ))
    );
    let item_done = event_of(&items, "response.output_item.done");
    assert_eq!(text(at(item_done, &["item", "id"])), "msg_a");
    assert_eq!(integer(field(item_done, "sequence_number")), 1);
    let completed = items.last().unwrap();
    assert_eq!(kind(completed), "response.completed");
    assert_eq!(text(at(completed, &["response", "status"])), "completed");
    assert_eq!(text(at(completed, &["response", "id"])), "resp_a");
    let completed_message = item_at(at(completed, &["response", "output"]), 0);
    let completed_part = item_at(field(completed_message, "content"), 0);
    assert_eq!(text(field(completed_part, "text")), "abcdef");
    assert_eq!(transformed, original);
}

#[test]
fn status_events_are_deep_copies_with_in_progress_status() {
    let output = format!("[{}]", message_with_text("msg_a", "ok"));
    let extra = concat!(
        r#","error":{"code":"previous"},"instructions":"be nice","model":"gpt-4o","#,
        r#""metadata":{"trace":"t1"},"usage":{"input_tokens":2,"output_tokens":3,"total_tokens":5,"cost":0.5}"#
    );
    let mut transformed = response_with(&output, extra);
    let original = transformed.clone();
    let items = build(&mut transformed, None, 2);
    assert_in_progress_event(event_of(&items, "response.created"));
    assert_in_progress_event(event_of(&items, "response.in_progress"));
    let completed = items.last().unwrap();
    let completed_output = at(completed, &["response", "output"]);
    let message = item_at(completed_output, 0);
    let content = field(message, "content");
    assert_eq!(text(field(item_at(content, 0), "text")), "ok");
    assert_eq!(transformed, original);
}

#[test]
fn added_payloads_keep_complete_content() {
    let output = concat!(
        r#"[{"type":"message","id":"msg_a","status":"completed","role":"assistant","content":["#,
        r#"{"type":"output_text","text":"abcdef","annotations":[],"logprobs":null},"#,
        r#"{"type":"refusal","refusal":"not today"}]}]"#
    );
    let mut transformed = response(output);
    let items = build(&mut transformed, None, 5);
    let item_added = event_of(&items, "response.output_item.added");
    let item = field(item_added, "item");
    assert_eq!(text(field(item, "type")), "message");
    assert_eq!(text(field(item, "role")), "assistant");
    assert_eq!(text(field(item, "status")), "completed");
    let content = field(item, "content");
    assert_eq!(text(field(item_at(content, 0), "text")), "abcdef");
    assert_eq!(text(field(item_at(content, 1), "refusal")), "not today");
    let first_part = event_of(&items, "response.content_part.added");
    assert_eq!(text(at(first_part, &["part", "text"])), "abcdef");
    let part_dones = events_of(&items, "response.content_part.done");
    assert_eq!(part_dones.len(), 2);
    assert_eq!(text(at(part_dones[0], &["part", "text"])), "abcdef");
    assert_eq!(text(at(part_dones[1], &["part", "refusal"])), "not today");
    let item_done = event_of(&items, "response.output_item.done");
    assert_eq!(field(item_done, "item"), item);
}

#[test]
fn annotations_come_after_all_deltas_and_before_text_done() {
    let output = concat!(
        r#"[{"type":"message","id":"msg_a","status":"completed","role":"assistant","content":["#,
        r#"{"type":"output_text","text":"abcd","annotations":["#,
        r#"{"type":"url_citation","url":"https://example.test/a","title":"A","start_index":0,"end_index":1,"extra":2},"#,
        r#"{"type":"file_path","file_id":"file_1","index":3}"#,
        r#"],"logprobs":null}]}]"#
    );
    let mut transformed = response(output);
    let items = build(&mut transformed, None, 2);
    assert_eq!(
        kinds(&items),
        [
            "response.created",
            "response.in_progress",
            "response.output_item.added",
            "response.content_part.added",
            "response.output_text.delta",
            "response.output_text.delta",
            "response.output_text.annotation.added",
            "response.output_text.annotation.added",
            "response.output_text.done",
            "response.content_part.done",
            "response.output_item.done",
            "response.completed",
        ]
    );
    let last_delta = items
        .iter()
        .rposition(|event| kind(event) == "response.output_text.delta")
        .expect("deltas present");
    let first_annotation = position(&items, "response.output_text.annotation.added");
    assert!(first_annotation > last_delta);
    assert!(first_annotation < position(&items, "response.output_text.done"));
    let annotations = events_of(&items, "response.output_text.annotation.added");
    assert_eq!(integer(field(annotations[0], "annotation_index")), 0);
    assert_eq!(integer(field(annotations[1], "annotation_index")), 1);
    assert_eq!(
        text(at(annotations[0], &["annotation", "url"])),
        "https://example.test/a"
    );
    assert_eq!(integer(at(annotations[0], &["annotation", "extra"])), 2);
    assert_eq!(
        text(at(annotations[1], &["annotation", "file_id"])),
        "file_1"
    );
}

#[test]
fn part_done_defaults_annotations_and_logprobs() {
    let output = r#"[{"type":"message","id":"msg_a","status":"completed","role":"assistant","content":[{"type":"output_text","text":"hi"}]}]"#;
    let mut transformed = response(output);
    let items = build(&mut transformed, None, 5);
    let part_done = event_of(&items, "response.content_part.done");
    assert_eq!(
        field(part_done, "part"),
        &json(r#"{"type":"output_text","text":"hi","annotations":[],"logprobs":null}"#)
    );
    let part_added = event_of(&items, "response.content_part.added");
    assert_eq!(
        field(part_added, "part"),
        &json(r#"{"type":"output_text","text":"hi"}"#)
    );
    let logprobs_output = concat!(
        r#"[{"type":"message","id":"msg_a","status":"completed","role":"assistant","content":["#,
        r#"{"type":"output_text","text":"hi","annotations":[],"#,
        r#""logprobs":[{"token":"hi","logprob":-0.5,"bytes":[104,105],"top_logprobs":[]}]}]}]"#
    );
    let mut transformed = response(logprobs_output);
    let items = build(&mut transformed, None, 5);
    let part_done = event_of(&items, "response.content_part.done");
    assert_eq!(
        at(part_done, &["part", "logprobs"]),
        &json(r#"[{"token":"hi","logprob":-0.5,"bytes":[104,105],"top_logprobs":[]}]"#)
    );
}

#[test]
fn empty_text_emits_done_without_deltas() {
    let output = format!("[{}]", message_with_text("msg_a", ""));
    let mut transformed = response(&output);
    let items = build(&mut transformed, None, 3);
    assert!(events_of(&items, "response.output_text.delta").is_empty());
    let text_done = event_of(&items, "response.output_text.done");
    assert_eq!(text(field(text_done, "text")), "");
    assert_eq!(
        text(at(
            event_of(&items, "response.content_part.done"),
            &["part", "text"]
        )),
        ""
    );
}

#[test]
fn chunks_split_by_unicode_characters() {
    let output = format!("[{}]", message_with_text("msg_a", "aé😀b"));
    let mut transformed = response(&output);
    let items = build(&mut transformed, None, 1);
    assert_eq!(
        deltas(&items, "response.output_text.delta"),
        ["a", "é", "😀", "b"]
    );
    let mut transformed = response(&output);
    let items = build(&mut transformed, None, 2);
    assert_eq!(deltas(&items, "response.output_text.delta"), ["aé", "😀b"]);
}

#[test]
fn zero_chunk_size_raises_range_error() {
    let output = format!("[{}]", message_with_text("msg_a", "abc"));
    let mut transformed = response(&output);
    let error =
        build_synthetic_response_events(&mut transformed, None::<&FixedCost>, 0).unwrap_err();
    assert!(matches!(error, SyntheticEventsError::ZeroChunkSize));
    assert_eq!(error.to_string(), "range() arg 3 must not be zero");
    let output = r#"[{"type":"function_call","id":"fc_1","call_id":"call_1","name":"f","arguments":"","status":"completed"}]"#;
    let mut transformed = response(output);
    let error =
        build_synthetic_response_events(&mut transformed, None::<&FixedCost>, 0).unwrap_err();
    assert!(matches!(error, SyntheticEventsError::ZeroChunkSize));
}

#[test]
fn negative_chunk_size_emits_no_deltas() {
    let output = format!("[{}]", message_with_text("msg_a", "abc"));
    let mut transformed = response(&output);
    let items = build(&mut transformed, None, -2);
    assert!(events_of(&items, "response.output_text.delta").is_empty());
    assert_eq!(
        text(field(event_of(&items, "response.output_text.done"), "text")),
        "abc"
    );
    assert_eq!(
        text(at(
            event_of(&items, "response.content_part.done"),
            &["part", "text"]
        )),
        "abc"
    );
    let output = r#"[{"type":"function_call","id":"fc_1","call_id":"call_1","name":"f","arguments":"{}","status":"completed"}]"#;
    let mut transformed = response(output);
    let items = build(&mut transformed, None, -1);
    assert!(events_of(&items, "response.function_call_arguments.delta").is_empty());
    assert_eq!(
        text(field(
            event_of(&items, "response.function_call_arguments.done"),
            "arguments"
        )),
        "{}"
    );
}

#[test]
fn event_budget_charges_events_and_bytes_up_to_each_limit() {
    let mut budget = EventBudget::default();
    budget.charge(MAX_SYNTHETIC_EVENTS, 0).unwrap();
    assert!(matches!(
        budget.charge(1, 0),
        Err(SyntheticEventsError::EventBudgetExceeded)
    ));
    let mut budget = EventBudget::default();
    budget.charge(0, MAX_SYNTHETIC_BYTES).unwrap();
    assert!(matches!(
        budget.charge(0, 1),
        Err(SyntheticEventsError::EventBudgetExceeded)
    ));
}

#[test]
fn oversized_item_id_with_tiny_chunks_raises_the_event_budget() {
    let item_id = "i".repeat(64 * 1024);
    let text = "t".repeat(64 * 1024);
    let output = format!("[{}]", message_with_text(&item_id, &text));
    let mut transformed = response(&output);
    let error =
        build_synthetic_response_events(&mut transformed, None::<&FixedCost>, 1).unwrap_err();
    assert!(matches!(error, SyntheticEventsError::EventBudgetExceeded));
    assert_eq!(
        error.to_string(),
        "synthetic response events would exceed the size budget"
    );
}

#[test]
fn zero_chunk_size_without_text_events_succeeds() {
    let mut transformed = response("[]");
    let items = build(&mut transformed, None, 0);
    assert_eq!(
        kinds(&items),
        [
            "response.created",
            "response.in_progress",
            "response.completed"
        ]
    );
}
