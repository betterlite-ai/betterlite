use rstest::rstest;
use serde_json::Number;

use super::fake_services::FakeServices;
use super::support::{
    cached_entry, embedding, extra, kwargs, largest_count, process, try_process, usage,
};
use crate::cache_key::value::KeyValue;
use crate::embedding_merge::error::{MergeError, ProcessError};
use crate::types::embedding::CachedEmbedding;

fn entry_with_details(tokens: i64, details: Option<&str>) -> CachedEmbedding {
    CachedEmbedding {
        prompt_tokens_details: details.map(extra),
        ..cached_entry(Some("a"), Some(tokens))
    }
}

#[rstest]
#[case::reads_the_cached_count(Some(7), "hello", 7, &[])]
#[case::a_zero_count_is_a_count(Some(0), "hello", 0, &[])]
#[case::counts_a_string_input_without_a_count(None, "hello", 5, &["hello"])]
#[case::counts_an_empty_string_input(None, "", 0, &[""])]
#[tokio::test]
async fn counts_prompt_tokens_only_for_entries_lacking_them(
    #[case] cached_tokens: Option<i64>,
    #[case] input: &str,
    #[case] expected_tokens: i64,
    #[case] counted: &[&str],
) {
    let services = FakeServices::working();
    let mut kwargs = kwargs(&[input]);
    let cached_result = [Some(cached_entry(Some("a"), cached_tokens))];
    let outcome = process(&cached_result, &mut kwargs, None, &services).await;
    let response = outcome.response.expect("something was cached");
    assert_eq!(
        response.usage,
        Some(usage([expected_tokens, 0, expected_tokens], None))
    );
    assert_eq!(*services.counted.lock().expect("the lock is held"), counted);
}

#[tokio::test]
async fn does_not_count_a_token_list_input() {
    let services = FakeServices::working();
    let mut kwargs = kwargs(&[]);
    let tokens = KeyValue::Array(vec![
        KeyValue::Number(Number::from(1_u8)),
        KeyValue::Number(Number::from(2_u8)),
    ]);
    kwargs.insert("input".to_owned(), KeyValue::Array(vec![tokens]));
    let cached_result = [Some(cached_entry(Some("a"), None))];
    let outcome = process(&cached_result, &mut kwargs, None, &services).await;
    let response = outcome.response.expect("something was cached");
    assert_eq!(response.usage, Some(usage([0, 0, 0], None)));
    assert!(
        services
            .counted
            .lock()
            .expect("the lock is held")
            .is_empty()
    );
}

#[tokio::test]
async fn counts_each_old_entry_on_its_own_text_and_never_an_uncached_one() {
    let services = FakeServices::working();
    let mut kwargs = kwargs(&["one", "skipped", "three", "four"]);
    let cached_result = [
        Some(cached_entry(Some("a"), None)),
        None,
        Some(cached_entry(Some("c"), None)),
        Some(cached_entry(Some("d"), Some(10))),
    ];
    let outcome = process(&cached_result, &mut kwargs, None, &services).await;
    let response = outcome.response.expect("something was cached");
    assert_eq!(response.usage, Some(usage([18, 0, 18], None)));
    assert_eq!(
        *services.counted.lock().expect("the lock is held"),
        ["one", "three"]
    );
}

#[tokio::test]
async fn passes_a_counting_failure_on() {
    let services = FakeServices::failing_to_count();
    let mut kwargs = kwargs(&["hello"]);
    let cached_result = [Some(cached_entry(Some("a"), None))];
    let error = try_process(&cached_result, &mut kwargs, None, &services)
        .await
        .expect_err("the counter fails");
    assert!(matches!(error, ProcessError::Service { .. }));
}

#[tokio::test]
async fn reports_an_overflow_of_the_prompt_token_total() {
    let services = FakeServices::working();
    let mut kwargs = kwargs(&["i0", "i1"]);
    let cached_result = [
        Some(cached_entry(Some("a"), Some(i64::MAX))),
        Some(cached_entry(Some("b"), Some(1))),
    ];
    let error = try_process(&cached_result, &mut kwargs, None, &services)
        .await
        .expect_err("the total does not fit");
    assert!(matches!(
        error,
        ProcessError::Merge {
            source: MergeError::Overflow {
                field: "prompt_tokens"
            }
        }
    ));
}

#[tokio::test]
async fn sums_the_totals_up_to_the_largest_count() {
    let services = FakeServices::working();
    let mut kwargs = kwargs(&["i0", "i1"]);
    let cached_result = [
        Some(cached_entry(Some("a"), Some(i64::MAX - 1))),
        Some(cached_entry(Some("b"), Some(1))),
    ];
    let outcome = process(&cached_result, &mut kwargs, None, &services).await;
    let response = outcome.response.expect("something was cached");
    assert_eq!(
        response.usage.map(|total| total.prompt_tokens),
        Some(i64::MAX)
    );
}

#[rstest]
#[case::numbers_add(
    r#"{"cached_tokens": 1, "text_tokens": 2}"#,
    r#"{"cached_tokens": 3}"#,
    Some(r#"{"cached_tokens": 4, "text_tokens": 2}"#)
)]
#[case::floats_add(
    r#"{"video_length_seconds": 1.5}"#,
    r#"{"video_length_seconds": 2.25}"#,
    Some(r#"{"video_length_seconds": 3.75}"#)
)]
#[case::a_later_text_replaces_an_earlier_one(
    r#"{"note": "a"}"#,
    r#"{"note": "b"}"#,
    Some(r#"{"note": "b"}"#)
)]
#[case::a_later_text_replaces_an_earlier_number(
    r#"{"note": 1}"#,
    r#"{"note": "b"}"#,
    Some(r#"{"note": "b"}"#)
)]
#[case::a_later_number_replaces_an_earlier_text(
    r#"{"note": "a"}"#,
    r#"{"note": 2}"#,
    Some(r#"{"note": 2}"#)
)]
#[case::a_field_with_a_rejected_type_drops_the_details(
    r#"{"cached_tokens": 1}"#,
    r#"{"cached_tokens": "x"}"#,
    None
)]
#[tokio::test]
async fn aggregates_the_prompt_token_details_of_the_cached_entries(
    #[case] first: &str,
    #[case] second: &str,
    #[case] expected: Option<&str>,
) {
    let services = FakeServices::working();
    let mut kwargs = kwargs(&["i0", "i1"]);
    let cached_result = [
        Some(entry_with_details(1, Some(first))),
        Some(entry_with_details(2, Some(second))),
    ];
    let outcome = process(&cached_result, &mut kwargs, None, &services).await;
    let total = outcome.response.and_then(|response| response.usage);
    assert_eq!(total, Some(usage([3, 0, 3], expected)));
}

#[rstest]
#[case::no_details(None, None)]
#[case::empty_details(Some("{}"), None)]
#[case::only_nulls_leave_empty_details(Some(r#"{"cached_tokens": null}"#), Some("{}"))]
#[tokio::test]
async fn leaves_the_details_out_when_no_entry_has_any(
    #[case] details: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let services = FakeServices::working();
    let mut kwargs = kwargs(&["i0"]);
    let cached_result = [Some(entry_with_details(1, details))];
    let outcome = process(&cached_result, &mut kwargs, None, &services).await;
    let total = outcome.response.and_then(|response| response.usage);
    assert_eq!(total, Some(usage([1, 0, 1], expected)));
}

#[tokio::test]
async fn skips_the_details_of_uncached_positions() {
    let services = FakeServices::working();
    let mut kwargs = kwargs(&["i0", "i1"]);
    let cached_result = [
        None,
        Some(entry_with_details(2, Some(r#"{"cached_tokens": 5}"#))),
    ];
    let outcome = process(&cached_result, &mut kwargs, None, &services).await;
    let response = outcome.response.expect("something was cached");
    assert_eq!(
        response.usage,
        Some(usage([2, 0, 2], Some(r#"{"cached_tokens": 5}"#)))
    );
    assert_eq!(response.data, [None, Some(embedding("a", 1))]);
}

#[tokio::test]
async fn reports_an_overflow_of_the_detail_totals() {
    let services = FakeServices::working();
    let mut kwargs = kwargs(&["i0", "i1"]);
    let mut first = entry_with_details(1, None);
    first.prompt_tokens_details = Some(largest_count());
    let cached_result = [
        Some(first),
        Some(entry_with_details(1, Some(r#"{"cached_tokens": 1}"#))),
    ];
    let error = try_process(&cached_result, &mut kwargs, None, &services)
        .await
        .expect_err("the detail total does not fit");
    assert!(matches!(
        error,
        ProcessError::Merge {
            source: MergeError::Overflow {
                field: "prompt_tokens_details"
            }
        }
    ));
}
