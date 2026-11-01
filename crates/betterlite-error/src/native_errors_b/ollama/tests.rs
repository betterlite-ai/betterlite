#![cfg(test)]

use rstest::rstest;

use super::*;

fn dict_input(error: &str) -> OllamaExceptionInput {
    OllamaExceptionInput {
        model: "ollama/some-model".to_owned(),
        original_exception: OllamaOriginalException::Dict {
            error: error.to_owned(),
        },
    }
}

fn text_input(text: &str) -> OllamaExceptionInput {
    OllamaExceptionInput {
        model: "ollama/some-model".to_owned(),
        original_exception: OllamaOriginalException::Text {
            text: text.to_owned(),
        },
    }
}

fn family(class: &ErrorClass) -> &'static str {
    match class {
        ErrorClass::BadRequest(_) => "bad_request",
        ErrorClass::ServiceUnavailable(_) => "service_unavailable",
        ErrorClass::Timeout(_) => "timeout",
        ErrorClass::Authentication(_)
        | ErrorClass::NotFound(_)
        | ErrorClass::PermissionDenied(_)
        | ErrorClass::UnprocessableEntity(_)
        | ErrorClass::RateLimit(_)
        | ErrorClass::BudgetExceeded(_)
        | ErrorClass::ContextWindowExceeded(_)
        | ErrorClass::ContentPolicyViolation(_)
        | ErrorClass::BadGateway(_)
        | ErrorClass::InternalServer(_)
        | ErrorClass::Api(_)
        | ErrorClass::ApiConnection(_)
        | ErrorClass::ApiResponseValidation(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::UnsupportedParams(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::MidStreamFallback(_)
        | ErrorClass::Unclassified(_) => panic!("ollama classification never produces {class:?}"),
    }
}

#[rstest]
#[case::no_such_file(text_input("no such file or directory"), Some("bad_request"))]
#[case::connection_refused(
    text_input("Failed to establish a new connection"),
    Some("service_unavailable")
)]
#[case::invalid_response(text_input("Invalid response object from API"), Some("bad_request"))]
#[case::read_timed_out(text_input("Read timed out"), Some("timeout"))]
#[case::unmatched_text_falls_through(text_input("boom"), None)]
#[case::dict_error_field_is_the_match_target(dict_input("Read timed out"), Some("timeout"))]
#[case::dict_without_error_key_falls_through(dict_input(""), None)]
fn native_errors_b_behavior_tests_behavior(
    #[case] input: OllamaExceptionInput,
    #[case] expected_family: Option<&str>,
) {
    let mapped = map_ollama_exception(&input);
    match expected_family {
        None => assert!(
            mapped.is_none(),
            "expected no classification, got {mapped:?}"
        ),
        Some(expected) => {
            let class = mapped.expect("a matching predicate must produce a class");
            assert_eq!(family(&class), expected);
        }
    }
}

#[test]
fn a_dict_original_exception_ignores_its_str_form() {
    // isinstance(original_exception, dict) reads the "error" key, never str(original_exception)
    let dict = dict_input("Read timed out");
    let text = text_input("completely unrelated text");
    assert!(map_ollama_exception(&dict).is_some());
    assert!(map_ollama_exception(&text).is_none());
}
