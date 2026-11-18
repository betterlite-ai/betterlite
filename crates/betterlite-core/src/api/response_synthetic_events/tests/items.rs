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
    let empty_output = r#"[{"type":"function_call","id":"fc_1","call_id":"call_1","name":"f","arguments":"","status":"completed"}]"#;
    let mut empty_transformed = response(empty_output);
    let empty_items = build(&mut empty_transformed, None, 5);
    assert!(events_of(&empty_items, "response.function_call_arguments.delta").is_empty());
    assert_eq!(
        text(field(
            event_of(&empty_items, "response.function_call_arguments.done"),
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
fn unknown_item_with_summary_emits_only_item_events() {
    let output =
        r#"[{"type":"mystery","id":"m_1","summary":[{"type":"summary_text","text":"x"}]}]"#;
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
fn many_empty_summaries_with_a_huge_item_id_exceed_the_event_budget() {
    let item_id = "i".repeat(64 * 1024);
    // 2500 empty summaries pass the byte limit at two id copies each, so the done charge counts
    let summaries = [r#"{"type":"summary_text","text":""}"#; 2_500].join(",");
    let output = format!(r#"[{{"type":"reasoning","id":"{item_id}","summary":[{summaries}]}}]"#);
    let mut transformed = response(&output);
    let error =
        build_synthetic_response_events(&mut transformed, None::<&FixedCost>, 1).unwrap_err();
    assert!(matches!(error, SyntheticEventsError::EventBudgetExceeded));
}

#[test]
fn many_empty_function_calls_with_a_huge_response_id_exceed_the_event_budget() {
    // every call falls back to the 1 MiB response id and clones it into its done event
    let call = r#"{"type":"function_call","id":"","call_id":"call_1","name":"f","arguments":"","status":"completed"}"#;
    let calls = [call; 300].join(",");
    let mut transformed = response(&format!("[{calls}]"));
    transformed.id = "r".repeat(1024 * 1024);
    let error =
        build_synthetic_response_events(&mut transformed, None::<&FixedCost>, 1).unwrap_err();
    assert!(matches!(error, SyntheticEventsError::EventBudgetExceeded));
}

#[test]
fn oversized_output_is_rejected_before_it_is_copied() {
    const CHILD_ENV: &str = "BETTERLITE_MEASURE_SYNTHETIC_OUTPUT_COPY";
    if std::env::var_os(CHILD_ENV).is_none() {
        let executable = std::env::current_exe().unwrap();
        let status = std::process::Command::new(executable)
            .args([
                "--exact",
                "api::response_synthetic_events::tests::items::oversized_output_is_rejected_before_it_is_copied",
                "--nocapture",
            ])
            .env(CHILD_ENV, "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }

    let input = "x".repeat(90 * 1024 * 1024);
    let output = format!(
        r#"[{{"type":"custom_tool_call","id":"ct_1","call_id":"call_1","name":"g","input":"{input}","status":"completed"}}]"#
    );
    let mut transformed = response(&output);
    drop(output);
    drop(input);
    let peak_before = peak_resident_kib();
    let error =
        build_synthetic_response_events(&mut transformed, None::<&FixedCost>, 1).unwrap_err();
    let peak_after = peak_resident_kib();
    assert!(matches!(error, SyntheticEventsError::EventBudgetExceeded));
    assert_eq!(transformed.output.len(), 1);
    assert!(peak_after - peak_before < 32 * 1024);
}

#[test]
fn oversized_envelope_keeps_output_after_budget_error() {
    let output = format!("[{}]", message_with_text("msg_a", "still present"));
    let mut transformed = response(&output);
    let original_output = transformed.output.clone();
    transformed.instructions = Some("i".repeat(90 * 1024 * 1024));

    let error =
        build_synthetic_response_events(&mut transformed, None::<&FixedCost>, 1).unwrap_err();

    assert!(matches!(error, SyntheticEventsError::EventBudgetExceeded));
    assert_eq!(transformed.output, original_output);
}

fn peak_resident_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmHWM:")
                .and_then(|value| value.split_whitespace().next())
                .and_then(|value| value.parse().ok())
        })
        .unwrap()
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
    let message_output = format!("[{}]", message_with_text("msg_a", "hi"));
    let mut message_transformed = parse::<ResponsesApiResponse>(&format!(
        r#"{{"id":"resp_a","created_at":1700000000,"status":"incomplete","output":{message_output}}}"#
    ));
    let message_items = build(&mut message_transformed, None, 1);
    assert_eq!(kind(message_items.last().unwrap()), "response.completed");
    assert_eq!(
        text(at(message_items.last().unwrap(), &["response", "status"])),
        "incomplete"
    );
}
