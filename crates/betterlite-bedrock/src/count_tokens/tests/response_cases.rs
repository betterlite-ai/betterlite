use crate::count_tokens::response::transform_response;

use super::support::{assert_result, request};

pub(super) fn count_to_anthropic_result() {
    assert_result(
        &transform_response(&request(r#"{"inputTokens": 123}"#)),
        r#"{"input_tokens": 123}"#,
    );
    assert_result(
        &transform_response(&request("{}")),
        r#"{"input_tokens": 0}"#,
    );
    assert_result(
        &transform_response(&request(r#"{"inputTokens": null}"#)),
        r#"{"input_tokens": null}"#,
    );
    assert_result(
        &transform_response(&request(r#"{"inputTokens": 0}"#)),
        r#"{"input_tokens": 0}"#,
    );
}
