#![cfg(test)]

mod cases;
mod corpus;
mod support;

use rstest::rstest;

#[rstest]
#[case::canvas_images_array(cases::canvas_images_array)]
#[case::canvas_error_object(cases::canvas_error_object)]
#[case::canvas_empty_images(cases::canvas_empty_images)]
#[case::canvas_malformed_reply(cases::canvas_malformed_reply)]
#[case::canvas_response_cost(cases::canvas_response_cost)]
fn canvas_results_cases(#[case] case: fn()) {
    case();
}
