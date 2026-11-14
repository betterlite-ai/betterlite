use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::langfuse_handler::{LangFuseHandler, LangfuseSelectionError};
use crate::langfuse_logger::tests::support as common;

use super::{dynamic, text};

#[rstest]
#[case::host("langfuse_host")]
#[case::public_key("langfuse_public_key")]
#[case::secret("langfuse_secret")]
#[case::secret_key("langfuse_secret_key")]
fn any_present_field_counts(#[case] field: &str) {
    let settings = common::settings(&[]);
    let params = dynamic(vec![(field, text("value"))]);
    assert!(LangFuseHandler::has_dynamic_credentials(&settings, &params).expect("the fields read"));
}

#[test]
fn absent_or_null_fields_do_not_count() {
    let settings = common::settings(&[]);
    let nulls = dynamic(vec![
        ("langfuse_host", Json::Null),
        ("langfuse_secret", Json::Null),
    ]);
    assert!(!LangFuseHandler::has_dynamic_credentials(&settings, &nulls).expect("the fields read"));
    assert!(
        !LangFuseHandler::has_dynamic_credentials(&settings, &dynamic(vec![]))
            .expect("the fields read")
    );
}

#[test]
fn a_meaningful_environment_counts() {
    let settings = common::settings(&[]);
    let params = dynamic(vec![("langfuse_environment", text("staging"))]);
    assert!(LangFuseHandler::has_dynamic_credentials(&settings, &params).expect("the fields read"));
}

#[test]
fn params_that_are_not_an_object_are_refused() {
    let settings = common::settings(&[]);
    let error = LangFuseHandler::has_dynamic_credentials(&settings, &text("x"))
        .expect_err("the params are refused");
    assert_eq!(
        error.to_string(),
        "standard_callback_dynamic_params must be an object"
    );
    assert!(matches!(error, LangfuseSelectionError::ParamsType));
}
