use indexmap::IndexMap;

use super::support::{Harness, input_wire, row_usage};
use super::{KEY_CALLER, triples};

// LiteLLM hands a null body.model to the counter as None; our counter takes text, so the empty name is used
#[test]
fn a_null_model_counts_with_the_empty_model_name() {
    let harness = Harness::new(r#"{"body": {"model": null, "prompt": "hello"}}"#);
    harness.counter.script(&[5]);
    let result = harness
        .run(&input_wire("file-abc", Some(KEY_CALLER), "[]"))
        .expect("the row accounts");
    assert_eq!(result.total_tokens, 5);
    assert_eq!(result.request_count, 1);
    assert_eq!(result.output_tokens, 0);
    assert!(result.per_model_usage.is_empty());
    assert_eq!(harness.counter.shapes(), triples(&[("text", "", "hello")]));
    assert!(harness.access.asked().is_empty());
}

// A body.model that is not text names no model, and the counter refuses the row, so its size estimate applies
#[test]
fn a_non_string_model_makes_the_row_unmeasurable() {
    let harness = Harness::new(r#"{"body": {"model": 5, "prompt": "hello"}}"#);
    let result = harness
        .run(&input_wire("file-abc", Some(KEY_CALLER), "[]"))
        .expect("the row accounts");
    assert_eq!(result.total_tokens, 10);
    assert_eq!(result.request_count, 1);
    assert_eq!(result.output_tokens, 0);
    assert!(result.per_model_usage.is_empty());
    assert!(harness.counter.calls().is_empty());
    assert!(harness.access.asked().is_empty());
}

// Python integers are unbounded; the i64 totals stop at i64::MAX instead of wrapping
#[test]
fn huge_counts_saturate_at_i64_max() {
    let row = r#"{"body": {"model": "gpt-4o", "prompt": "hello"}}"#;
    let harness = Harness::new(&format!("{row}\n{row}"));
    harness.counter.script(&[usize::MAX, usize::MAX]);
    harness.calls.script(&[i64::MAX, i64::MAX]);
    let result = harness
        .run(&input_wire("file-abc", Some(KEY_CALLER), "[]"))
        .expect("the rows account");
    assert_eq!(result.total_tokens, i64::MAX);
    assert_eq!(result.output_tokens, i64::MAX);
    assert_eq!(result.request_count, 2);
    assert_eq!(
        result.per_model_usage,
        IndexMap::from([("gpt-4o".to_owned(), row_usage(i64::MAX, i64::MAX, 2))])
    );
    assert!(harness.logs.silence());
}
