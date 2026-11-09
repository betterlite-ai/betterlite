use rstest::rstest;

use super::fixtures::{call_kwargs, ids, object, payload, sources};
use crate::fields::payload::StandardLoggingPayload;
use crate::newrelic_extraction::response_fields::{
    Usage, completion_id, finish_reason, model_names, usage, vendor,
};

const GENERATED: &str = "00000000-0000-0000-0000-000000000007";

#[rstest]
#[case::response_id(r#"{"id":"chatcmpl-9"}"#, None, "chatcmpl-9")]
#[case::number_id(r#"{"id":5}"#, Some("call-1"), "5")]
#[case::empty_id_falls_back(r#"{"id":""}"#, Some("call-1"), "call-1")]
#[case::missing_id_falls_back("{}", Some("call-1"), "call-1")]
#[case::empty_call_id_generates("{}", Some(""), GENERATED)]
#[case::nothing_available("{}", None, GENERATED)]
fn newrelic_extraction_tests_behavior_completion_id(
    #[case] response: &str,
    #[case] call_id: Option<&str>,
    #[case] expected: &str,
) {
    let body = object(response);
    let mut kwargs = call_kwargs();
    kwargs.litellm_call_id = call_id;
    let event_sources = sources(&kwargs, &body, None);
    assert_eq!(completion_id(&event_sources, &ids(7)), expected);
}

#[test]
fn newrelic_extraction_tests_behavior_vendor_prefers_the_payload() {
    let mut standard = payload();
    standard.custom_llm_provider = Some("openai".to_owned());
    let body = object("{}");
    let mut kwargs = call_kwargs();
    kwargs.custom_llm_provider = Some("azure");
    let event_sources = sources(&kwargs, &body, Some(&standard));
    assert_eq!(vendor(&event_sources), "openai");
}

#[test]
fn newrelic_extraction_tests_behavior_vendor_falls_back_to_kwargs() {
    let standard = payload();
    let body = object("{}");
    let mut kwargs = call_kwargs();
    kwargs.custom_llm_provider = Some("azure");
    let event_sources = sources(&kwargs, &body, Some(&standard));
    assert_eq!(vendor(&event_sources), "azure");
}

#[test]
fn newrelic_extraction_tests_behavior_vendor_defaults_to_litellm() {
    let body = object("{}");
    let kwargs = call_kwargs();
    let event_sources = sources(&kwargs, &body, None);
    assert_eq!(vendor(&event_sources), "litellm");
}

#[test]
fn newrelic_extraction_tests_behavior_vendor_defaults_on_empty_values() {
    let mut standard = payload();
    standard.custom_llm_provider = Some(String::new());
    let body = object("{}");
    let mut kwargs = call_kwargs();
    kwargs.custom_llm_provider = Some("");
    let event_sources = sources(&kwargs, &body, Some(&standard));
    assert_eq!(vendor(&event_sources), "litellm");
}

fn names_of(
    standard: Option<&StandardLoggingPayload>,
    model: Option<&str>,
    response: &str,
) -> (String, String) {
    let body = object(response);
    let mut kwargs = call_kwargs();
    kwargs.model = model;
    let event_sources = sources(&kwargs, &body, standard);
    model_names(&event_sources)
}

#[rstest]
#[case::payload_and_response_models(r#"{"model":"gpt-4o-2024-08-06"}"#, "gpt-4o", ("gpt-4o", "gpt-4o-2024-08-06"))]
#[case::response_model_missing("{}", "gpt-4o", ("gpt-4o", "gpt-4o"))]
#[case::response_model_empty(r#"{"model":""}"#, "gpt-4o", ("gpt-4o", "gpt-4o"))]
#[case::response_model_number(r#"{"model":5}"#, "gpt-4o", ("gpt-4o", "5"))]
fn newrelic_extraction_tests_behavior_model_names_from_payload(
    #[case] response: &str,
    #[case] payload_model: &str,
    #[case] expected: (&str, &str),
) {
    let mut standard = payload();
    standard.model = payload_model.to_owned();
    let names = names_of(Some(&standard), Some("kwargs-model"), response);
    assert_eq!(names, (expected.0.to_owned(), expected.1.to_owned()));
}

#[rstest]
#[case::kwargs_model(Some("gpt-4o-mini"), r#"{"model":"gpt-4o-mini-2024"}"#, ("gpt-4o-mini", "gpt-4o-mini-2024"))]
#[case::unknown_model(None, "{}", ("unknown", "unknown"))]
#[case::empty_kwargs_model(Some(""), "{}", ("unknown", "unknown"))]
fn newrelic_extraction_tests_behavior_model_names_fall_back_to_kwargs(
    #[case] model: Option<&str>,
    #[case] response: &str,
    #[case] expected: (&str, &str),
) {
    let mut standard = payload();
    standard.model = String::new();
    let names = names_of(Some(&standard), model, response);
    assert_eq!(names, (expected.0.to_owned(), expected.1.to_owned()));
}

#[test]
fn newrelic_extraction_tests_behavior_model_names_without_payload() {
    let names = names_of(None, None, "{}");
    assert_eq!(names, ("unknown".to_owned(), "unknown".to_owned()));
}

fn usage_of(standard: Option<&StandardLoggingPayload>, response: &str) -> Usage {
    let body = object(response);
    let kwargs = call_kwargs();
    let event_sources = sources(&kwargs, &body, standard);
    usage(&event_sources)
}

#[test]
fn newrelic_extraction_tests_behavior_usage_prefers_the_payload() {
    let standard = payload();
    let counts = usage_of(
        Some(&standard),
        r#"{"usage":{"prompt_tokens":9,"completion_tokens":9,"total_tokens":9}}"#,
    );
    assert_eq!(
        counts,
        Usage {
            prompt_tokens: 2,
            completion_tokens: 3,
            total_tokens: 5
        }
    );
}

#[test]
fn newrelic_extraction_tests_behavior_usage_keeps_payload_zeros() {
    let mut standard = payload();
    standard.prompt_tokens = 0;
    standard.completion_tokens = 0;
    standard.total_tokens = 0;
    let counts = usage_of(
        Some(&standard),
        r#"{"usage":{"prompt_tokens":9,"completion_tokens":9,"total_tokens":9}}"#,
    );
    assert_eq!(counts, Usage::default());
}

#[test]
fn newrelic_extraction_tests_behavior_usage_reads_the_response() {
    let counts = usage_of(
        None,
        r#"{"usage":{"prompt_tokens":4,"completion_tokens":6,"total_tokens":10}}"#,
    );
    assert_eq!(
        counts,
        Usage {
            prompt_tokens: 4,
            completion_tokens: 6,
            total_tokens: 10
        }
    );
}

#[test]
fn newrelic_extraction_tests_behavior_usage_defaults_without_usage() {
    assert_eq!(usage_of(None, "{}"), Usage::default());
}

#[test]
fn newrelic_extraction_tests_behavior_usage_defaults_missing_counts() {
    let counts = usage_of(None, r#"{"usage":{"prompt_tokens":4}}"#);
    assert_eq!(
        counts,
        Usage {
            prompt_tokens: 4,
            completion_tokens: 0,
            total_tokens: 0
        }
    );
}

#[test]
fn newrelic_extraction_tests_behavior_usage_ignores_non_numeric_counts() {
    let counts = usage_of(
        None,
        r#"{"usage":{"prompt_tokens":"4","completion_tokens":true,"total_tokens":null}}"#,
    );
    assert_eq!(counts, Usage::default());
}

#[rstest]
#[case::first_choice(r#"{"choices":[{"finish_reason":"stop"}]}"#, "stop")]
#[case::empty_reason(r#"{"choices":[{"finish_reason":""}]}"#, "unknown")]
#[case::missing_reason(r#"{"choices":[{}]}"#, "unknown")]
#[case::empty_choices(r#"{"choices":[]}"#, "unknown")]
#[case::no_choices("{}", "unknown")]
#[case::choices_not_list(r#"{"choices":{}}"#, "unknown")]
#[case::choice_not_object(r#"{"choices":[7]}"#, "unknown")]
#[case::number_reason(r#"{"choices":[{"finish_reason":5}]}"#, "5")]
fn newrelic_extraction_tests_behavior_finish_reason(
    #[case] response: &str,
    #[case] expected: &str,
) {
    let body = object(response);
    let kwargs = call_kwargs();
    let event_sources = sources(&kwargs, &body, None);
    assert_eq!(finish_reason(&event_sources), expected);
}
