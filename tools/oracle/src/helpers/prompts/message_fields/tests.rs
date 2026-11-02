#![cfg(test)]

use super::{Input, Op};
use crate::helpers::Helper as _;

#[test]
fn missing_file_field_carries_status_400() {
    let input: Input =
        serde_json::from_str(r#"{"operation":"migrate_file_to_image_url","part":{"type":"file"}}"#)
            .unwrap();
    let error = Op::run(&input).unwrap_err();
    assert_eq!(error.class, "BadRequestError");
    assert_eq!(error.status, Some(400));
}

#[test]
fn missing_file_reference_keeps_the_value_error_class() {
    let input: Input = serde_json::from_str(
        r#"{"operation":"migrate_file_to_image_url","part":{"type":"file","file":{"format":"image/png"}}}"#,
    )
    .unwrap();
    let error = Op::run(&input).unwrap_err();
    assert_eq!(error.class, "ValueError");
    assert_eq!(error.status, None);
}
