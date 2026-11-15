use super::support::{resolves_none_with, supports_response_schema};

const NON_SUPPORTING_ONLY: &str =
    r#"{"azure/unknown-none": {"supports_none_reasoning_effort": true}}"#;
const WITH_ANOTHER_DEFAULT: &str = r#"{"azure/unknown-none": {"supports_none_reasoning_effort": true}, "azure/other": {"default_reasoning_effort": "medium"}}"#;

#[test]
fn azure_catalogue_default() {
    assert!(resolves_none_with(NON_SUPPORTING_ONLY, "unknown-none"));
    assert!(!resolves_none_with(WITH_ANOTHER_DEFAULT, "unknown-none"));
}

#[test]
fn azure_response_schema_forward() {
    assert!(supports_response_schema("azure/gpt-5.1"));
    assert!(!supports_response_schema("azure/o1-preview"));
}
