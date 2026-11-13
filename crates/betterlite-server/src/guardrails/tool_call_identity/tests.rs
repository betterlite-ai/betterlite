#![cfg(test)]

use std::collections::BTreeSet;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use rstest::rstest;
use serde_json::{Value, json};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use super::error::UndeliverableRewriteReason;
use super::{
    ToolCallShape, UndeliverableRewriteInput, post_guardrail_shapes, tool_call_rewrite,
    tool_call_shapes, undeliverable_reason,
};

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}

fn shape(name: Option<&str>, arguments: &str) -> ToolCallShape {
    ToolCallShape {
        name: name.map(str::to_owned),
        arguments: arguments.to_owned(),
    }
}

fn call(name: &str, arguments: &str) -> Value {
    json!({"function": {"name": name, "arguments": arguments}})
}

#[test]
fn shape_reads_name_and_arguments() {
    let calls = [call("lookup", "{\"city\":\"Oslo\"}")];
    assert_eq!(
        tool_call_shapes(&calls),
        [shape(Some("lookup"), "{\"city\":\"Oslo\"}")]
    );
}

#[test]
fn shape_keeps_the_call_order() {
    let calls = [call("first", "1"), call("second", "2")];
    let names: Vec<Option<String>> = tool_call_shapes(&calls)
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    assert_eq!(names, [Some("first".to_owned()), Some("second".to_owned())]);
}

#[rstest]
#[case::missing_name(json!({"function": {"arguments": "a"}}), None, "a")]
#[case::null_name(json!({"function": {"name": null, "arguments": "a"}}), None, "a")]
#[case::number_name(json!({"function": {"name": 4, "arguments": "a"}}), None, "a")]
#[case::missing_arguments(json!({"function": {"name": "n"}}), Some("n"), "")]
#[case::null_arguments(json!({"function": {"name": "n", "arguments": null}}), Some("n"), "")]
#[case::number_arguments(json!({"function": {"name": "n", "arguments": 4}}), Some("n"), "")]
#[case::empty_arguments(json!({"function": {"name": "n", "arguments": ""}}), Some("n"), "")]
fn shape_reads_each_field_leniently(
    #[case] tool_call: Value,
    #[case] name: Option<&str>,
    #[case] arguments: &str,
) {
    assert_eq!(
        tool_call_shapes(&[tool_call]),
        [shape(name, arguments)],
        "a non-string field keeps its empty shape"
    );
}

#[rstest]
#[case::not_an_object(json!("tool"))]
#[case::missing_function(json!({"id": "c1"}))]
#[case::null_function(json!({"function": null}))]
#[case::string_function(json!({"function": "lookup"}))]
fn shape_never_fails_on_a_call_without_a_function_object(#[case] tool_call: Value) {
    assert_eq!(tool_call_shapes(&[tool_call]), [shape(None, "")]);
}

#[test]
fn shape_of_an_empty_list_is_empty() {
    assert!(tool_call_shapes(&[]).is_empty());
}

#[test]
fn post_guardrail_returns_the_validated_shapes() {
    let pre = tool_call_shapes(&[call("lookup", "a")]);
    let returned = [call("lookup", "b")];
    assert_eq!(
        post_guardrail_shapes(Some(&returned), &pre, Some("pii")),
        [shape(Some("lookup"), "b")]
    );
}

#[test]
fn post_guardrail_accepts_a_null_name_and_missing_arguments() {
    let pre = tool_call_shapes(&[call("lookup", "a")]);
    let returned = [json!({"function": {"name": null}})];
    assert_eq!(
        post_guardrail_shapes(Some(&returned), &pre, None),
        [shape(None, "")]
    );
}

#[test]
fn post_guardrail_ignores_other_returned_keys() {
    let pre = tool_call_shapes(&[call("lookup", "a")]);
    let returned =
        [json!({"id": "x", "function": {"name": "lookup", "arguments": "b", "extra": 1}})];
    assert_eq!(
        post_guardrail_shapes(Some(&returned), &pre, None),
        [shape(Some("lookup"), "b")]
    );
}

#[test]
fn post_guardrail_empty_pre_returns_early_without_a_warning() {
    let (logs, _guard) = capture_logs();
    let returned = [call("added", "a")];
    assert!(post_guardrail_shapes(Some(&returned), &[], Some("pii")).is_empty());
    assert_eq!(logs.contents(), "");
}

#[test]
fn post_guardrail_missing_returned_falls_back_with_a_warning() {
    let (logs, _guard) = capture_logs();
    let pre = tool_call_shapes(&[call("lookup", "a")]);
    assert_eq!(post_guardrail_shapes(None, &pre, Some("pii")), pre);
    assert!(logs.contents().contains("pii"));
    assert!(logs.contents().contains("returned a different number"));
}

#[test]
fn post_guardrail_length_mismatch_falls_back_with_a_warning() {
    let (logs, _guard) = capture_logs();
    let pre = tool_call_shapes(&[call("lookup", "a")]);
    let returned = [call("lookup", "a"), call("other", "b")];
    assert_eq!(post_guardrail_shapes(Some(&returned), &pre, None), pre);
    assert!(logs.contents().contains("returned a different number"));
}

#[rstest]
#[case::null_arguments(json!({"function": {"name": "lookup", "arguments": null}}))]
#[case::number_arguments(json!({"function": {"name": "lookup", "arguments": 3}}))]
#[case::number_name(json!({"function": {"name": 3, "arguments": "a"}}))]
#[case::missing_function(json!({"name": "lookup", "arguments": "a"}))]
#[case::function_is_a_string(json!({"function": "lookup"}))]
#[case::not_an_object(json!("lookup"))]
fn post_guardrail_any_invalid_call_falls_back_with_a_warning(#[case] returned_call: Value) {
    let (logs, _guard) = capture_logs();
    let pre = tool_call_shapes(&[call("lookup", "a")]);
    assert_eq!(
        post_guardrail_shapes(Some(&[returned_call]), &pre, Some("pii")),
        pre
    );
    assert!(
        logs.contents()
            .contains("without a function name and arguments")
    );
}

#[test]
fn post_guardrail_one_invalid_call_drops_the_whole_list() {
    let pre = tool_call_shapes(&[call("first", "a"), call("second", "b")]);
    let returned = [
        call("first", "changed"),
        json!({"function": {"name": "second", "arguments": null}}),
    ];
    assert_eq!(post_guardrail_shapes(Some(&returned), &pre, None), pre);
}

#[test]
fn rewrite_keeps_a_changed_name() {
    let before = shape(Some("lookup"), "a");
    let after = shape(Some("search"), "a");
    assert_eq!(
        tool_call_rewrite(&before, &after),
        shape(Some("search"), "a")
    );
}

#[rstest]
#[case::same_name(Some("lookup"), Some("lookup"))]
#[case::both_missing(None, None)]
fn rewrite_drops_an_unchanged_name(#[case] before: Option<&str>, #[case] after: Option<&str>) {
    let rewrite = tool_call_rewrite(&shape(before, "a"), &shape(after, "a"));
    assert_eq!(rewrite, shape(None, "a"));
}

#[rstest]
#[case::added(None, Some("lookup"))]
#[case::removed(Some("lookup"), None)]
fn rewrite_keeps_a_name_that_turned_some_or_none(
    #[case] before: Option<&str>,
    #[case] after: Option<&str>,
) {
    let rewrite = tool_call_rewrite(&shape(before, "a"), &shape(after, "a"));
    assert_eq!(rewrite.name, after.map(str::to_owned));
}

#[test]
fn rewrite_always_keeps_the_new_arguments() {
    let rewrite = tool_call_rewrite(&shape(Some("lookup"), "old"), &shape(Some("lookup"), "new"));
    assert_eq!(rewrite.arguments, "new");
}

fn ids(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn undeliverable_missing_call_id_comes_first() {
    let reason = undeliverable_reason(&UndeliverableRewriteInput {
        call_ids: &["c1".to_owned()],
        tool_call_item_count: 2,
        post_guardrail_tool_call_count: 0,
        unresolved_argument_event: true,
        rewritten_call_ids: &ids(&["c1"]),
        event_call_ids: &BTreeSet::new(),
    });
    assert_eq!(
        reason,
        Some(UndeliverableRewriteReason::MissingCallId {
            missing: 1,
            total: 2
        })
    );
    assert_eq!(
        reason.unwrap().to_string(),
        "1 of the stream's 2 tool call items carry no call_id"
    );
}

#[test]
fn undeliverable_repeated_call_id_comes_second() {
    let reason = undeliverable_reason(&UndeliverableRewriteInput {
        call_ids: &["c1".to_owned(), "c1".to_owned()],
        tool_call_item_count: 2,
        post_guardrail_tool_call_count: 0,
        unresolved_argument_event: true,
        rewritten_call_ids: &ids(&["c1"]),
        event_call_ids: &BTreeSet::new(),
    });
    assert_eq!(reason, Some(UndeliverableRewriteReason::RepeatedCallId));
    assert_eq!(
        reason.unwrap().to_string(),
        "the stream's tool call items repeat a call_id"
    );
}

#[test]
fn undeliverable_call_count_mismatch_comes_third() {
    let reason = undeliverable_reason(&UndeliverableRewriteInput {
        call_ids: &["c1".to_owned()],
        tool_call_item_count: 1,
        post_guardrail_tool_call_count: 2,
        unresolved_argument_event: true,
        rewritten_call_ids: &ids(&["c1"]),
        event_call_ids: &BTreeSet::new(),
    });
    assert_eq!(
        reason,
        Some(UndeliverableRewriteReason::CallCountMismatch {
            returned: 2,
            expected: 1
        })
    );
    assert_eq!(
        reason.unwrap().to_string(),
        "the guardrail returned 2 tool calls for the stream's 1 tool call items"
    );
}

#[test]
fn undeliverable_unresolved_argument_event_comes_fourth() {
    let reason = undeliverable_reason(&UndeliverableRewriteInput {
        call_ids: &["c1".to_owned()],
        tool_call_item_count: 1,
        post_guardrail_tool_call_count: 1,
        unresolved_argument_event: true,
        rewritten_call_ids: &ids(&["c1"]),
        event_call_ids: &BTreeSet::new(),
    });
    assert_eq!(
        reason,
        Some(UndeliverableRewriteReason::UnresolvedArgumentEvent)
    );
}

#[test]
fn undeliverable_missing_event_call_ids_come_last_and_sorted() {
    let reason = undeliverable_reason(&UndeliverableRewriteInput {
        call_ids: &["c1".to_owned(), "c2".to_owned()],
        tool_call_item_count: 2,
        post_guardrail_tool_call_count: 2,
        unresolved_argument_event: false,
        rewritten_call_ids: &ids(&["c2", "c1"]),
        event_call_ids: &ids(&["c2"]),
    });
    assert_eq!(
        reason,
        Some(UndeliverableRewriteReason::MissingEventCallId {
            missing: vec!["c1".to_owned()]
        })
    );
    assert_eq!(
        reason.unwrap().to_string(),
        "no stream event carries the rewritten call_id c1"
    );
}

#[test]
fn undeliverable_lists_every_missing_event_call_id_sorted() {
    let reason = undeliverable_reason(&UndeliverableRewriteInput {
        call_ids: &["b".to_owned(), "a".to_owned(), "c".to_owned()],
        tool_call_item_count: 3,
        post_guardrail_tool_call_count: 3,
        unresolved_argument_event: false,
        rewritten_call_ids: &ids(&["c", "a", "b"]),
        event_call_ids: &BTreeSet::new(),
    });
    assert_eq!(
        reason,
        Some(UndeliverableRewriteReason::MissingEventCallId {
            missing: vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]
        })
    );
    assert_eq!(
        reason.unwrap().to_string(),
        "no stream event carries the rewritten call_id a, b, c"
    );
}

#[test]
fn undeliverable_is_none_when_every_check_passes() {
    assert_eq!(
        undeliverable_reason(&UndeliverableRewriteInput {
            call_ids: &["c1".to_owned(), "c2".to_owned()],
            tool_call_item_count: 2,
            post_guardrail_tool_call_count: 2,
            unresolved_argument_event: false,
            rewritten_call_ids: &ids(&["c1"]),
            event_call_ids: &ids(&["c1", "c2"]),
        }),
        None
    );
}

#[test]
fn undeliverable_reports_the_first_failing_check() {
    let reason = undeliverable_reason(&UndeliverableRewriteInput {
        call_ids: &["c1".to_owned(), "c1".to_owned()],
        tool_call_item_count: 2,
        post_guardrail_tool_call_count: 1,
        unresolved_argument_event: true,
        rewritten_call_ids: &ids(&["c1"]),
        event_call_ids: &BTreeSet::new(),
    });
    assert_eq!(reason, Some(UndeliverableRewriteReason::RepeatedCallId));
}
