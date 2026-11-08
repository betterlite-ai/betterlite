#![cfg(test)]

use betterlite_config::sections::mock_response::failure::MockError;
use betterlite_config::sections::mock_response::{MockChatObject, MockResponse};
use betterlite_openai::embedding::EmbeddingVector;
use rstest::rstest;
use serde_json::Number;

use super::{DEFAULT_DIMENSIONS, response};

fn numbers(values: &[f64]) -> Vec<Number> {
    values
        .iter()
        .map(|value| Number::from_f64(*value).expect("finite number"))
        .collect()
}

#[rstest]
#[case::given_vector(numbers(&[0.1_f64, 0.2_f64]))]
#[case::empty_vector(numbers(&[]))]
fn mock_calls_behavior_tests_behavior(#[case] values: Vec<Number>) {
    let value = MockResponse::Numbers(values.clone());
    let answer = response(Some(&value), "text-embedding-3-small").expect("mock embedding");
    assert_eq!(answer.model, "text-embedding-3-small");
    assert_eq!(answer.object, "list");
    let record = answer.data.first().expect("embedding");
    assert_eq!(record.object, "embedding");
    assert_eq!(record.index, 0);
    let EmbeddingVector::Floats(actual) = &record.embedding else {
        panic!("expected float values");
    };
    assert_eq!(actual, &values);
    assert_eq!(answer.usage.prompt_tokens, 10);
    assert_eq!(answer.usage.completion_tokens, 0);
    assert_eq!(answer.usage.total_tokens, 0);
}

#[test]
fn a_missing_mock_response_fills_the_default_vector() {
    let answer = response(None, "text-embedding-3-small").expect("mock embedding");
    let record = answer.data.first().expect("embedding");
    let EmbeddingVector::Floats(values) = &record.embedding else {
        panic!("expected float values");
    };
    assert_eq!(values.len(), DEFAULT_DIMENSIONS);
    assert!(values.iter().all(|value| value.as_f64() == Some(0.0_f64)));
}

#[test]
fn the_error_sentinel_raises_the_mock_failure() {
    let value = MockResponse::Text(String::from("error"));
    assert_eq!(
        response(Some(&value), "m").err(),
        Some(MockError::EmbeddingFailure)
    );
}

#[rstest]
#[case::text("text")]
#[case::object("object")]
fn a_value_that_does_not_fit_is_a_typed_failure(#[case] kind: &str) {
    let value = match kind {
        "text" => MockResponse::Text(String::from("hello")),
        _ => MockResponse::Object(MockChatObject {
            id: None,
            created: None,
            model: None,
            object: None,
            system_fingerprint: None,
            service_tier: None,
            choices: None,
            usage: None,
        }),
    };
    assert_eq!(
        response(Some(&value), "m").err(),
        Some(MockError::WrongType {
            field: "mock_response"
        })
    );
}
