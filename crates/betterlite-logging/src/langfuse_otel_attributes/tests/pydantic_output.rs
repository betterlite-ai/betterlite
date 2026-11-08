#![cfg(test)]

use super::output_content::get_output_content_by_type;
use super::support::value;
use crate::langfuse_projections::content::ResponseObj;

#[test]
fn model_response_dumps_its_json() {
    let body = value(r#"{"id":"chatcmpl-1","choices":[{"message":{"role":"assistant"}}]}"#);
    assert_eq!(
        get_output_content_by_type(Some(&ResponseObj::Chat(&body)), Some("completion")),
        r#"{"id":"chatcmpl-1","choices":[{"message":{"role":"assistant"}}]}"#
    );
}

#[test]
fn an_empty_model_still_dumps_its_json() {
    let body = value("{}");
    assert_eq!(
        get_output_content_by_type(Some(&ResponseObj::ResponsesApi(&body)), None),
        "{}"
    );
}
