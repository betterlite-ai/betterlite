use rstest::rstest;

use crate::watsonx_auth::error::WatsonxError;
use crate::watsonx_auth::urls::deployment_id;

#[rstest]
#[case::plain("deployment/abc", Some("abc"))]
#[case::slashes("deployment/a/b", Some("a/b"))]
#[case::query("deployment/x?y#z", Some("x%3Fy%23z"))]
#[case::not_a_deployment("granite", None)]
#[case::prefix_needs_the_slash("deployment", None)]
fn deployment_id_keeps_plain_ids(#[case] model: &str, #[case] expected: Option<&str>) {
    assert_eq!(deployment_id(model).unwrap().as_deref(), expected);
}

#[rstest]
#[case::parent("deployment/x/../other")]
#[case::current("deployment/./x")]
#[case::empty("deployment/")]
#[case::empty_inner("deployment/a//b")]
fn deployment_id_refuses_a_path_escape(#[case] model: &str) {
    assert!(matches!(
        deployment_id(model).unwrap_err(),
        WatsonxError::Api {
            status_code: 400,
            ..
        }
    ));
}
