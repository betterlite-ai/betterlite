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

#[rstest::rstest]
#[case::query("query", 400, "invalid-query-parameter")]
#[case::body("body", 422, "invalid-request-body")]
fn validation_location_selects_problem(
    #[case] source: &str,
    #[case] status: u16,
    #[case] slug: &str,
) {
    let error = ValidationErrorDetail {
        error_type: "too_short".to_owned(),
        loc: vec![
            LocPart::Field(source.to_owned()),
            LocPart::Field("items".to_owned()),
        ],
        msg: "need one".to_owned(),
    };
    let problem = request_validation_problem(&[error]);
    assert_eq!(problem.status, status);
    assert_eq!(problem.r#type, format!("urn:litellm:error:{slug}"));
    assert_eq!(problem.detail, "items: need one");
}

#[test]
fn empty_validation_uses_query_fallback() {
    let problem = request_validation_problem(&[]);
    assert_eq!(problem.status, 400);
    assert_eq!(problem.detail, "The request query parameters are invalid.");
}

#[test]
fn unknown_query_uses_only_explicit_aliases() {
    let declared = std::collections::BTreeSet::from(["alias"]);
    assert!(super::reject_unknown_query_params(&["alias".to_owned()], &declared).is_ok());
    let error =
        super::reject_unknown_query_params(&["internal_name".to_owned()], &declared).unwrap_err();
    assert_eq!(error.0.status, 400);
    assert_eq!(error.0.r#type, "urn:litellm:error:unknown-query-parameter");
    assert_eq!(error.0.allowed, Some(vec!["alias".to_owned()]));
}

#[test]
fn like_metacharacters_are_escaped_once() {
    assert_eq!(super::escape_like(r"key\_%"), r"key\\\_\%");
}

#[test]
fn remaining_body_error_takes_precedence_over_query() {
    let query = ValidationErrorDetail {
        error_type: "int_parsing".to_owned(),
        loc: vec![
            LocPart::Field("query".to_owned()),
            LocPart::Field("page".to_owned()),
        ],
        msg: "integer required".to_owned(),
    };
    let body = ValidationErrorDetail {
        error_type: "missing".to_owned(),
        loc: vec![
            LocPart::Field("body".to_owned()),
            LocPart::Field("name".to_owned()),
        ],
        msg: "name required".to_owned(),
    };
    let problem = request_validation_problem(&[query, body]);
    assert_eq!(problem.status, 422);
    assert_eq!(
        problem.detail,
        "page: integer required; name: name required"
    );
}
