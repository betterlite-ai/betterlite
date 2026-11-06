#![cfg(test)]

use crate::presidio_actions::error::PresidioActionsError;
use crate::presidio_actions::{PresidioEndpoints, validate_environment};

fn missing(_name: &str) -> Option<String> {
    None
}

fn from_environment(name: &str) -> Option<String> {
    match name {
        "PRESIDIO_ANALYZER_API_BASE" => Some("analyzer.example:8080".to_owned()),
        "PRESIDIO_ANONYMIZER_API_BASE" => Some("https://anonymizer.example".to_owned()),
        _ => None,
    }
}

fn only_analyzer(name: &str) -> Option<String> {
    (name == "PRESIDIO_ANALYZER_API_BASE").then(|| "analyzer.example:8080".to_owned())
}

fn empty_environment(name: &str) -> Option<String> {
    matches!(
        name,
        "PRESIDIO_ANALYZER_API_BASE" | "PRESIDIO_ANONYMIZER_API_BASE"
    )
    .then(String::new)
}

#[test]
fn arguments_are_normalized_with_scheme_and_trailing_slash() {
    let endpoints = validate_environment(
        Some("localhost:5001"),
        Some("https://anonymizer.local/"),
        from_environment,
    )
    .unwrap();
    assert_eq!(
        endpoints,
        PresidioEndpoints {
            analyzer_api_base: "http://localhost:5001/".to_owned(),
            anonymizer_api_base: "https://anonymizer.local/".to_owned(),
        }
    );
}

#[test]
fn environment_fills_missing_arguments() {
    let endpoints = validate_environment(None, None, from_environment).unwrap();
    assert_eq!(
        endpoints,
        PresidioEndpoints {
            analyzer_api_base: "http://analyzer.example:8080/".to_owned(),
            anonymizer_api_base: "https://anonymizer.example/".to_owned(),
        }
    );
}

#[test]
fn empty_arguments_fall_back_to_the_environment() {
    let endpoints = validate_environment(Some(""), Some(""), from_environment).unwrap();
    assert_eq!(endpoints.analyzer_api_base, "http://analyzer.example:8080/");
    assert_eq!(endpoints.anonymizer_api_base, "https://anonymizer.example/");
}

#[test]
fn missing_analyzer_base_is_an_error() {
    let error = validate_environment(None, Some("anonymizer.local"), missing).unwrap_err();
    assert!(matches!(
        error,
        PresidioActionsError::MissingAnalyzerApiBase
    ));
}

#[test]
fn missing_anonymizer_base_is_an_error() {
    let error = validate_environment(None, None, only_analyzer).unwrap_err();
    assert!(matches!(
        error,
        PresidioActionsError::MissingAnonymizerApiBase
    ));
}

#[test]
fn an_empty_environment_value_counts_as_unset() {
    let error = validate_environment(None, None, empty_environment).unwrap_err();
    assert!(matches!(
        error,
        PresidioActionsError::MissingAnalyzerApiBase
    ));
}
