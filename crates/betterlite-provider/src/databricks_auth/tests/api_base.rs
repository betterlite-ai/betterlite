use crate::databricks_auth::api_base::get_api_base;
use crate::databricks_auth::error::SDK_API_BASE;
use rstest::rstest;

#[test]
fn a_base_is_returned_untouched_when_the_gateway_is_off() {
    let base =
        get_api_base(Some("https://adb-1.databricks.net///"), false).expect("the base is present");
    assert_eq!(base, "https://adb-1.databricks.net///");
}

#[test]
fn a_missing_base_is_the_sdk_import_error() {
    let error = get_api_base(None, false).expect_err("a None base raises");
    assert_eq!(error.status_code, 400);
    assert_eq!(error.message, SDK_API_BASE);
    assert_eq!(error.headers, None);
}

#[rstest]
#[case::host_only(
    "https://adb-1.databricks.net",
    "https://adb-1.databricks.net/ai-gateway/mlflow/v1"
)]
#[case::root_path(
    "https://adb-1.databricks.net/",
    "https://adb-1.databricks.net/ai-gateway/mlflow/v1"
)]
#[case::many_slashes(
    "https://adb-1.databricks.net///",
    "https://adb-1.databricks.net/ai-gateway/mlflow/v1"
)]
#[case::query_only(
    "https://adb-1.databricks.net?x=1",
    "https://adb-1.databricks.net?x=1/ai-gateway/mlflow/v1"
)]
#[case::empty_base("", "/ai-gateway/mlflow/v1")]
fn the_gateway_appends_the_suffix_to_a_root_path(#[case] base: &str, #[case] expected: &str) {
    let resolved = get_api_base(Some(base), true).expect("the base is present");
    assert_eq!(resolved, expected);
}

#[rstest]
#[case::exact("https://adb-1.databricks.net/ai-gateway/mlflow/v1")]
#[case::trailing_slash("https://adb-1.databricks.net/ai-gateway/mlflow/v1/")]
fn the_gateway_keeps_an_ai_gateway_base(#[case] base: &str) {
    let resolved = get_api_base(Some(base), true).expect("the base is present");
    assert_eq!(
        resolved,
        "https://adb-1.databricks.net/ai-gateway/mlflow/v1"
    );
}

#[rstest]
#[case::plain("https://adb-1.databricks.net/serving-endpoints")]
#[case::trailing_slash("https://adb-1.databricks.net/serving-endpoints/")]
fn the_gateway_swaps_the_serving_endpoints_suffix(#[case] base: &str) {
    let resolved = get_api_base(Some(base), true).expect("the base is present");
    assert_eq!(
        resolved,
        "https://adb-1.databricks.net/ai-gateway/mlflow/v1"
    );
}

#[test]
fn another_path_keeps_its_trailing_slash() {
    let resolved = get_api_base(Some("https://adb-1.databricks.net/custom/"), true)
        .expect("the base is present");
    assert_eq!(resolved, "https://adb-1.databricks.net/custom/");
}
