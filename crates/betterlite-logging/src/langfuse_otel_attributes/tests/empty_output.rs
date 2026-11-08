#![cfg(test)]

use super::output_content::get_output_content_by_type;

#[test]
fn no_response_returns_the_empty_string() {
    assert_eq!(get_output_content_by_type(None, None), "");
}

#[test]
fn no_response_wins_over_a_completion_call_type() {
    assert_eq!(get_output_content_by_type(None, Some("completion")), "");
}
