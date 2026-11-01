#![cfg(test)]

use super::{Input, Op};
use crate::helpers::Helper as _;

#[test]
fn missing_file_field_keeps_the_bad_request_class() {
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

#[test]
fn migration_error_status_follows_the_variant() {
    let missing_file: Input =
        serde_json::from_str(r#"{"operation":"migrate_file_to_image_url","part":{"type":"file"}}"#)
            .unwrap();
    assert_eq!(Op::run(&missing_file).unwrap_err().status, Some(400));

    let missing_reference: Input = serde_json::from_str(
        r#"{"operation":"migrate_file_to_image_url","part":{"type":"file","file":{"format":"image/png"}}}"#,
    )
    .unwrap();
    assert_eq!(Op::run(&missing_reference).unwrap_err().status, None);
}
