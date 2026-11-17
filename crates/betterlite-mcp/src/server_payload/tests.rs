#![cfg(test)]

use betterlite_db::mcp::table_access::McpIdentifierConflict;
use betterlite_db::mcp::table_access::types::IdentifierField;
use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use super::{PayloadError, RequestError, quote, quoted_list};

#[test]
fn a_validator_error_displays_its_message_without_a_pydantic_prefix() {
    let error = RequestError::Value("args is required for stdio transport".to_owned());
    assert_eq!(error.to_string(), "args is required for stdio transport");
}

#[test]
fn a_shape_error_displays_the_serde_message() {
    let shape = serde_json::from_str::<Json>("{").unwrap_err();
    let expected = shape.to_string();
    assert_eq!(RequestError::Shape(shape).to_string(), expected);
}

#[test]
fn an_identifier_conflict_displays_litellms_message() {
    let error = PayloadError::IdentifierConflict {
        conflict: McpIdentifierConflict {
            field: IdentifierField::Alias,
            value: "taken".to_owned(),
            server_id: "srv-1".to_owned(),
        },
    };
    assert_eq!(
        error.to_string(),
        "An MCP server with alias 'taken' already exists (server_id=srv-1). MCP server names and aliases must be unique, case-insensitive."
    );
}

#[rstest]
#[case::plain("abc", "'abc'")]
#[case::single_quote_only("it's", "\"it's\"")]
#[case::double_quote_only("say \"hi\"", "'say \"hi\"'")]
#[case::both_quotes("a'b\"c", "'a\\'b\"c'")]
#[case::escapes("a\nb\tc\rd\\e", "'a\\nb\\tc\\rd\\\\e'")]
fn quote_follows_python_repr(#[case] value: &str, #[case] expected: &str) {
    assert_eq!(quote(value), expected);
}

#[test]
fn a_quoted_list_joins_the_reprs_in_order() {
    assert_eq!(quoted_list(["b", "a'"]), "['b', \"a'\"]");
    assert_eq!(quoted_list(Vec::<String>::new()), "[]");
}
