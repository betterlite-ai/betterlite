#![cfg(test)]

mod cases;
mod error_cases;
mod fakes;
mod logging_cases;
mod raised_cases;
mod support;

use rstest::rstest;

#[rstest]
#[case::string_and_token_input(0)]
#[case::list_input(1)]
#[case::dimensions_encoding(2)]
#[case::deployment_and_v1_path(3)]
#[case::embedding_usage(4)]
#[case::embedding_error(5)]
#[case::azure_embedding_client_types(6)]
#[case::azure_embedding_raw_parse(7)]
fn embeddings_cases(#[case] case: u8) {
    match case {
        0 => cases::string_and_token_input(),
        1 => cases::list_input(),
        2 => cases::dimensions_encoding(),
        3 => cases::deployment_and_v1_path(),
        4 => error_cases::embedding_usage(),
        5 => error_cases::embedding_error(),
        6 => error_cases::azure_embedding_client_types(),
        _ => error_cases::azure_embedding_raw_parse(),
    }
}

#[test]
fn embedding_logging_goes_through_the_logging_crate() {
    logging_cases::logs_pre_and_post_through_the_logging_crate();
}

#[test]
fn sdk_status_errors_name_the_body_like_the_sdk() {
    error_cases::status_messages();
}
