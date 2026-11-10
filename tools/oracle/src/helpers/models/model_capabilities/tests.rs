#![cfg(test)]

use rstest::rstest;

use super::Input;

#[rstest]
#[case::root_registry(r#"{"operation":"root_registry","source":{"providers":{}}}"#, true)]
#[case::missing_source_still_decodes(r#"{"operation":"root_registry"}"#, true)]
#[case::model_info_queries(
    r#"{"operation":"model_info","queries":[{"model":"gpt-4o","provider":"openai"}]}"#,
    true
)]
#[case::unknown_field(r#"{"operation":"packaged_registry","extra":true}"#, false)]
#[case::unknown_operation(r#"{"operation":"not_supported"}"#, false)]
#[case::query_without_model(
    r#"{"operation":"model_info","queries":[{"provider":"openai"}]}"#,
    false
)]
#[case::route_without_endpoint(
    r#"{"operation":"provider_endpoint","routes":[{"provider":"openai"}]}"#,
    false
)]
fn typed_capability_input(#[case] text: &str, #[case] valid: bool) {
    assert_eq!(serde_json::from_str::<Input>(text).is_ok(), valid);
}
