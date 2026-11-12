use rstest::rstest;
use serde_json::Number;

use super::fake_services::FakeServices;
use super::support::{
    cached_entry, embedding, extra, kwargs, previous_response, process, start, texts, try_process,
    usage,
};
use crate::cache_key::value::KeyValue;
use crate::embedding_merge::error::{MergeError, ProcessError};
use crate::types::embedding::CachedEmbedding;

#[tokio::test]
async fn splits_cached_and_uncached_positions_and_keeps_the_uncached_inputs() {
    let services = FakeServices::working();
    let mut kwargs = kwargs(&["i0", "i1", "i2"]);
    let cached_result = [
        Some(cached_entry(Some("a"), Some(3))),
        None,
        Some(cached_entry(Some("c"), Some(5))),
    ];
    let outcome = process(&cached_result, &mut kwargs, &services).await;
    let response = outcome.response.expect("something was cached");
    assert_eq!(
        response.data,
        [Some(embedding("a", 0)), None, Some(embedding("c", 2))]
    );
    assert_eq!(response.usage, Some(usage([8, 0, 8], None)));
    assert_eq!(kwargs["input"], texts(&["i1"]));
    assert!(!outcome.all_elements_cached);
    assert!(services.logged.lock().expect("the lock is held").is_empty());
}

#[tokio::test]
async fn marks_the_response_as_a_cache_hit_without_headers() {
    let services = FakeServices::working();
    let mut kwargs = kwargs(&["i0", "i1"]);
    let cached_result = [Some(cached_entry(Some("a"), Some(1))), None];
    let response = process(&cached_result, &mut kwargs, &services)
        .await
        .response
        .expect("something was cached");
    assert_eq!(response.hidden_params, extra(r#"{"cache_hit": true}"#));
    assert_eq!(response.response_headers, None);
}

#[tokio::test]
async fn an_entry_without_an_embedding_counts_as_cached_but_leaves_its_position_empty() {
    let services = FakeServices::working();
    let mut kwargs = kwargs(&["i0", "i1"]);
    let cached_result = [
        Some(cached_entry(None, Some(2))),
        Some(cached_entry(Some("b"), Some(1))),
    ];
    let outcome = process(&cached_result, &mut kwargs, &services).await;
    let response = outcome.response.expect("something was cached");
    assert_eq!(response.data, [None, Some(embedding("b", 1))]);
    assert_eq!(response.usage, Some(usage([3, 0, 3], None)));
    assert_eq!(kwargs["input"], texts(&[]));
    assert!(outcome.all_elements_cached);
}

#[tokio::test]
async fn logs_a_complete_hit_once_with_the_final_response() {
    let services = FakeServices::working();
    let mut kwargs = kwargs(&["i0", "i1"]);
    let cached_result = [
        Some(cached_entry(Some("a"), Some(1))),
        Some(cached_entry(Some("b"), Some(2))),
    ];
    let outcome = process(&cached_result, &mut kwargs, &services).await;
    assert!(outcome.all_elements_cached);
    assert_eq!(kwargs["input"], texts(&[]));
    let logged = services.logged.lock().expect("the lock is held");
    assert_eq!(logged.len(), 1);
    assert_eq!(logged[0].model, "request-model");
    assert_eq!(logged[0].start_time, start());
    assert_eq!(logged[0].kwargs["input"], texts(&[]));
    assert_eq!(Some(&logged[0].response), outcome.response.as_ref());
}

#[tokio::test]
async fn passes_the_previous_response_through_when_nothing_is_cached() {
    let services = FakeServices::working();
    let mut kwargs = kwargs(&["i0", "i1"]);
    let previous = previous_response();
    let outcome = try_process(
        &[None, None],
        &mut kwargs,
        Some(previous.clone()),
        &services,
    )
    .await
    .expect("the cached results are processed");
    assert_eq!(outcome.response, Some(previous));
    assert!(!outcome.all_elements_cached);
    assert_eq!(kwargs["input"], texts(&["i0", "i1"]));
    assert!(services.logged.lock().expect("the lock is held").is_empty());
    let none = process(&[None, None], &mut kwargs, &services).await;
    assert_eq!(none.response, None);
}

#[tokio::test]
async fn builds_a_new_response_instead_of_the_previous_one_when_something_is_cached() {
    let services = FakeServices::working();
    let mut kwargs = kwargs(&["i0", "i1"]);
    let previous = previous_response();
    let cached_result = [Some(cached_entry(Some("a"), Some(1))), None];
    let outcome = try_process(&cached_result, &mut kwargs, Some(previous), &services)
        .await
        .expect("the cached results are processed");
    let response = outcome.response.expect("something was cached");
    assert_eq!(response.model.as_deref(), Some("kwargs-model"));
}

#[rstest]
#[case::cached_model_wins([Some("cached-model"), None], Some("kwargs-model"), Some("cached-model"))]
#[case::first_cached_model_wins([Some("first"), Some("second")], Some("kwargs-model"), Some("first"))]
#[case::later_entry_names_the_model([None, Some("second")], Some("kwargs-model"), Some("second"))]
#[case::empty_cached_model_falls_back([Some(""), None], Some("kwargs-model"), Some("kwargs-model"))]
#[case::request_model_when_no_entry_names_one([None, None], Some("kwargs-model"), Some("kwargs-model"))]
#[case::no_model_anywhere([None, None], None, None)]
#[tokio::test]
async fn names_the_response_after_the_cached_model_or_the_request(
    #[case] models: [Option<&str>; 2],
    #[case] request_model: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let services = FakeServices::working();
    let mut kwargs = kwargs(&["i0", "i1"]);
    if request_model.is_none() {
        kwargs.shift_remove("model");
    }
    let cached_result = models.map(|model| {
        Some(CachedEmbedding {
            model: model.map(str::to_owned),
            ..cached_entry(Some("a"), Some(1))
        })
    });
    let outcome = process(&cached_result, &mut kwargs, &services).await;
    let response = outcome.response.expect("something was cached");
    assert_eq!(response.model.as_deref(), expected);
}

#[tokio::test]
async fn a_null_request_model_leaves_the_response_unnamed() {
    let services = FakeServices::working();
    let mut kwargs = kwargs(&["i0"]);
    kwargs.insert("model".to_owned(), KeyValue::Null);
    let cached_result = [Some(cached_entry(Some("a"), Some(1)))];
    let outcome = process(&cached_result, &mut kwargs, &services).await;
    assert_eq!(outcome.response.expect("a hit").model, None);
}

#[tokio::test]
async fn rejects_a_cached_list_of_another_length() {
    let services = FakeServices::working();
    let mut kwargs = kwargs(&["i0", "i1", "i2"]);
    let cached_result = [Some(cached_entry(Some("a"), Some(1))), None];
    let error = try_process(&cached_result, &mut kwargs, None, &services)
        .await
        .expect_err("two results cannot cover three inputs");
    assert!(matches!(
        error,
        ProcessError::Merge {
            source: MergeError::CachedLength {
                cached: 2,
                input: 3
            }
        }
    ));
    assert_eq!(kwargs["input"], texts(&["i0", "i1", "i2"]));
}

#[tokio::test]
async fn rejects_a_request_without_an_embedding_input() {
    let services = FakeServices::working();
    let mut kwargs = kwargs(&[]);
    kwargs.shift_remove("input");
    let error = try_process(&[], &mut kwargs, None, &services)
        .await
        .expect_err("there is no input to read");
    assert!(matches!(
        error,
        ProcessError::Merge {
            source: MergeError::Input { .. }
        }
    ));
}

#[tokio::test]
async fn rejects_a_request_model_that_is_not_a_string() {
    let services = FakeServices::working();
    let mut kwargs = kwargs(&["i0"]);
    kwargs.insert("model".to_owned(), KeyValue::Number(Number::from(5_u8)));
    let cached_result = [Some(cached_entry(Some("a"), Some(1)))];
    let error = try_process(&cached_result, &mut kwargs, None, &services)
        .await
        .expect_err("the model must be a string");
    assert!(matches!(
        error,
        ProcessError::Merge {
            source: MergeError::FieldType { field: "model", .. }
        }
    ));
}

#[tokio::test]
async fn passes_a_logging_failure_on() {
    let services = FakeServices::failing_to_log();
    let mut kwargs = kwargs(&["i0"]);
    let cached_result = [Some(cached_entry(Some("a"), Some(1)))];
    let error = try_process(&cached_result, &mut kwargs, None, &services)
        .await
        .expect_err("the log call fails");
    assert!(matches!(error, ProcessError::Service { .. }));
}

#[tokio::test]
async fn does_not_log_when_only_some_positions_are_cached_even_if_logging_fails() {
    let services = FakeServices::failing_to_log();
    let mut kwargs = kwargs(&["i0", "i1"]);
    let cached_result = [Some(cached_entry(Some("a"), Some(1))), None];
    let outcome = try_process(&cached_result, &mut kwargs, None, &services)
        .await
        .expect("a partial hit is not logged");
    assert!(!outcome.all_elements_cached);
}
