#![cfg(test)]

use super::output_content::get_output_content_by_type;
use super::support::value;
use crate::langfuse_projections::content::ResponseObj;

#[test]
fn dict_response_serializes_the_object() {
    let body = value(r#"{"answer":"ok"}"#);
    assert_eq!(
        get_output_content_by_type(Some(&ResponseObj::Dict(&body)), None),
        r#"{"answer":"ok"}"#
    );
}

#[test]
fn an_empty_dict_returns_the_empty_string() {
    let body = value("{}");
    assert_eq!(
        get_output_content_by_type(Some(&ResponseObj::Dict(&body)), None),
        ""
    );
}
