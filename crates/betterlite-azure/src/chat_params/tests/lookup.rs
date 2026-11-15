use super::support::resolves_none_with;

const NON_SUPPORTING_ONLY: &str =
    r#"{"azure/unknown-none": {"supports_none_reasoning_effort": true}}"#;
const WITH_ANOTHER_DEFAULT: &str = r#"{"azure/unknown-none": {"supports_none_reasoning_effort": true}, "azure/other": {"default_reasoning_effort": "medium"}}"#;

#[test]
fn azure_catalogue_default() {
    assert!(resolves_none_with(NON_SUPPORTING_ONLY, "unknown-none"));
    assert!(!resolves_none_with(WITH_ANOTHER_DEFAULT, "unknown-none"));
}
