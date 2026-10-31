#![cfg(test)]

use std::collections::BTreeMap;
use std::iter::repeat_n;
use std::{fs, path::PathBuf, str::from_utf8};

use crate::facts::presence::Field;
use crate::facts::{ProviderErrorBody, ResponseFacts};
use rstest::rstest;

use super::*;

#[rstest]
#[case::body_message(Some(ProviderErrorBody { message: Field::Value("direct".to_owned()), error: Field::Missing, code: Field::Missing }), Some("direct"))]
#[case::nested_message(Some(ProviderErrorBody { message: Field::Missing, error: Field::Value(Box::new(ProviderErrorBody { message: Field::Value("nested".to_owned()), error: Field::Missing, code: Field::Missing })), code: Field::Missing }), Some("nested"))]
#[case::missing_message(Some(ProviderErrorBody { message: Field::Missing, error: Field::Missing, code: Field::Missing }), None)]
#[case::empty_message_uses_nested(Some(ProviderErrorBody { message: Field::Value(String::new()), error: Field::Value(Box::new(ProviderErrorBody { message: Field::Value("nested".to_owned()), error: Field::Missing, code: Field::Missing })), code: Field::Missing }), Some("nested"))]
#[case::null_body(None, None)]
fn error_facts_behavior_tests_behavior(
    #[case] body: Option<ProviderErrorBody>,
    #[case] expected: Option<&str>,
) {
    let facts = ProviderErrorFacts {
        body,
        status_code: None,
        status_code_is_synthesized: false,
        message: None,
        headers: None,
        response: None,
        litellm_response_headers: None,
        request: None,
    };
    let message = get_error_message(&facts);
    assert_eq!(message.as_deref(), expected);
}

#[path = "tests/header_precedence/tests.rs"]
mod header_precedence;

#[path = "tests/header_fallback/tests.rs"]
mod header_fallback;

#[path = "tests/constructor_kwargs/tests.rs"]
mod constructor_kwargs;

#[path = "tests/body_code/tests.rs"]
mod body_code;

#[path = "tests/code_boundary/tests.rs"]
mod code_boundary;

#[path = "tests/exception_names/tests.rs"]
mod exception_names;

#[path = "tests/plain_constructors/tests.rs"]
mod plain_constructors;

#[path = "tests/missing_arguments/tests.rs"]
mod missing_arguments;

#[path = "tests/no_signature/tests.rs"]
mod no_signature;

#[path = "tests/parser_bounds/tests.rs"]
mod parser_bounds;

#[path = "tests/extraction/tests.rs"]
mod extraction;

#[rstest]
fn replays_the_provider_error_corpus(#[files("fuzz/corpus/provider_error/*")] path: PathBuf) {
    let malformed = include_bytes!("../../fuzz/corpus/provider_error/malformed");
    let nested = include_bytes!("../../fuzz/corpus/provider_error/nested");
    drop(parse_provider_error(malformed).unwrap_err());
    drop(parse_provider_error(nested).unwrap());
    let bytes = fs::read(path).unwrap();
    let _parsed = parse_provider_error(&bytes);
}

#[path = "tests/presence/tests.rs"]
mod presence;

#[path = "tests/code_fields/tests.rs"]
mod code_fields;

mod gate_boundary_cases;

mod gate_constructor_cases;
