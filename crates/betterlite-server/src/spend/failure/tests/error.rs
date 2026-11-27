use betterlite_error::normalize::Normalizer;
use betterlite_error::proxy_model_not_found::ProxyModelNotFoundError;
use betterlite_logging::logging_metadata::error_info::ExceptionLogging;
use betterlite_logging::logging_metadata::exception::ExceptionFacts;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::super::error::{FailureError, failure_error_information};
use super::super::metadata::as_object;
use super::support::{normalizer, policy};

fn logging<'facts>(
    facts: &'facts ExceptionFacts,
    normalizer: &'facts Normalizer,
) -> ExceptionLogging<'facts> {
    ExceptionLogging {
        exception: Some(facts),
        log_client_error_tracebacks: true,
        expected_client_error: false,
        redactor: None,
        normalizer,
    }
}

fn provider_facts() -> ExceptionFacts {
    ExceptionFacts {
        class_name: "ProviderError".to_owned(),
        message: Some("the provider is down".to_owned()),
        status_code: Some(503_u16),
        // ast-grep-ignore: no-default-struct-update - the test sets only the fields it reads
        ..ExceptionFacts::default()
    }
}

fn fields(information: &Json) -> &JsonObject {
    as_object(information).expect("the error information is an object")
}

#[test]
fn error_information_carries_the_class_code_traceback_and_message() {
    let normalizer = normalizer();
    let facts = provider_facts();
    let information = failure_error_information(&FailureError {
        logging: logging(&facts, &normalizer),
        model_not_found: None,
        traceback: Some("the stack"),
        policy: policy(false, 2048),
        suppress_traceback: false,
    })
    .expect("the information encodes");
    let fields = fields(&information);
    assert_eq!(
        fields["error_class"],
        Json::Text("ProviderError".to_owned())
    );
    assert_eq!(fields["error_code"], Json::Text("503".to_owned()));
    assert_eq!(
        fields["error_message"],
        Json::Text("the provider is down".to_owned())
    );
    assert_eq!(fields["traceback"], Json::Text("the stack".to_owned()));
}

#[test]
fn a_suppressed_traceback_loses_its_key_entirely() {
    let normalizer = normalizer();
    let facts = provider_facts();
    let information = failure_error_information(&FailureError {
        logging: logging(&facts, &normalizer),
        model_not_found: None,
        traceback: Some("the stack"),
        policy: policy(false, 2048),
        suppress_traceback: true,
    })
    .expect("the information encodes");
    assert!(!fields(&information).contains_key("traceback"));
}

#[test]
fn a_model_not_found_message_is_replaced_with_the_safe_one() {
    let normalizer = normalizer();
    let facts = provider_facts();
    let not_found = ProxyModelNotFoundError::new("/v1/chat/completions", "gpt-9", false);
    let information = failure_error_information(&FailureError {
        logging: logging(&facts, &normalizer),
        model_not_found: Some(&not_found),
        traceback: None,
        policy: policy(false, 2048),
        suppress_traceback: false,
    })
    .expect("the information encodes");
    assert_eq!(
        fields(&information)["error_message"],
        Json::Text(not_found.spend_log_error_message)
    );
}

#[test]
fn a_missing_exception_still_yields_an_object() {
    let normalizer = normalizer();
    let logging = ExceptionLogging {
        exception: None,
        log_client_error_tracebacks: true,
        expected_client_error: false,
        redactor: None,
        normalizer: &normalizer,
    };
    let information = failure_error_information(&FailureError {
        logging,
        model_not_found: None,
        traceback: None,
        policy: policy(false, 2048),
        suppress_traceback: false,
    })
    .expect("the information encodes");
    let fields = fields(&information);
    assert_eq!(fields["error_class"], Json::Text(String::new()));
    assert_eq!(fields["error_code"], Json::Text(String::new()));
}
