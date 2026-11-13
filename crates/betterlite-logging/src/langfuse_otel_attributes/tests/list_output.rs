#![cfg(test)]

use super::output_content::get_output_content_by_type;
use super::support::value;
use crate::langfuse_projections::content::ResponseObj;

#[test]
fn list_response_serializes_the_array() {
    let body = value(r#"[{"text":"one"}]"#);
    assert_eq!(
        get_output_content_by_type(Some(&ResponseObj::List(&body)), None),
        r#"[{"text":"one"}]"#
    );
}

#[test]
fn an_empty_list_returns_the_empty_string() {
    let body = value("[]");
    assert_eq!(
        get_output_content_by_type(Some(&ResponseObj::List(&body)), None),
        ""
    );
}
