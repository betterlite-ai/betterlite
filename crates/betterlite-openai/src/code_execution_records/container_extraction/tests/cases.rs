#[cfg(test)]
use rstest::rstest;
#[cfg(test)]
use serde_json::json;

#[cfg(test)]
use super::test_object;

#[cfg(test)]
#[rstest]
#[case::present(test_object([("id", json!("container-1"))]), Some("container-1".to_owned()))]
#[case::absent(json!({}), None)]
#[case::non_string(test_object([("id", json!(5_i64))]), None)]
fn container_id_reads_the_id_field(#[case] container: Value, #[case] expected: Option<String>) {
    assert_eq!(container_id(&container), expected);
}

#[cfg(test)]
#[rstest]
#[case::present(test_object([(SANDBOX_KEY, json!("abc"))]), Some("abc".to_owned()))]
#[case::absent(json!({}), None)]
#[case::non_string(test_object([(SANDBOX_KEY, json!(5_i64))]), None)]
fn sandbox_key_reads_the_sandbox_marker_from_kwargs(
    #[case] kwargs: Value,
    #[case] expected: Option<String>,
) {
    assert_eq!(sandbox_key(kwargs.as_object().unwrap()), expected);
}
