#![cfg(test)]

use rstest::rstest;

use super::Input;

#[rstest]
#[case::unknown(r#"{"group":"unknown"}"#)]
#[case::management(r#"{"group":"mcp_routes"}"#)]
#[case::missing("{}")]
#[case::non_string(r#"{"group":1}"#)]
#[case::extra_field(r#"{"group":"openai_routes","method":"POST"}"#)]
fn unknown_group(#[case] source: &str) {
    assert!(serde_json::from_str::<Input>(source).is_err());
}
