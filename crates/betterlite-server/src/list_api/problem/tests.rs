#![cfg(test)]

use super::{LocPart, ValidationErrorDetail, request_validation_problem};

#[test]
fn child_validation_error_suppresses_parent_too_short() {
    let errors = [
        ValidationErrorDetail {
            error_type: "too_short".to_owned(),
            loc: vec![
                LocPart::Field("body".to_owned()),
                LocPart::Field("items".to_owned()),
            ],
            msg: "need one".to_owned(),
        },
        ValidationErrorDetail {
            error_type: "string_type".to_owned(),
            loc: vec![
                LocPart::Field("body".to_owned()),
                LocPart::Field("items".to_owned()),
                LocPart::Index(0),
            ],
            msg: "not a string".to_owned(),
        },
    ];
    let problem = request_validation_problem(&errors);
    assert_eq!(problem.r#type, "urn:litellm:error:invalid-request-body");
    assert!(problem.detail.contains("items.0"));
    assert!(!problem.detail.contains("need one"));
}
