#![cfg(test)]

use rstest::rstest;

#[rstest]
#[case::schema("mcp-tool-calls/schema")]
#[case::absent("mcp-tool-calls/schema-absent")]
#[case::null("mcp-tool-calls/schema-null")]
#[case::explicit("mcp-tool-calls/schema-explicit")]
#[case::anthropic("mcp-tool-calls/schema-anthropic")]
#[case::json("mcp-tool-calls/openai-json-string-args")]
#[case::dict("mcp-tool-calls/openai-dict-args")]
#[case::invalid("mcp-tool-calls/openai-invalid-json-args")]
#[case::empty("mcp-tool-calls/openai-empty-args")]
#[case::missing("mcp-tool-calls/openai-missing-args")]
#[case::array("mcp-tool-calls/openai-array-args")]
#[case::looping("mcp-pagination/cursor-loop")]
#[case::exhaustion("mcp-pagination/page-exhaustion")]
#[case::deadline("mcp-pagination/deadline-expiration")]
#[case::empty_tools("mcp-pagination/empty-tools")]
#[case::slow("mcp-pagination/slow-single-page")]
#[case::openai("mcp-pagination/openai-load")]
#[case::override_deadline("mcp-pagination/deadline-override")]
fn replays_b4740_against_litellm(#[case] name: &str) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join(format!("cases/helpers/{name}.yaml"))).unwrap();
    let case = serde_saphyr::from_str::<crate::case::HelperCase>(&source).unwrap();
    let result = crate::helpers::run_case(&case, false).unwrap();
    assert!(matches!(result, crate::helpers::HelperRun::Match));
}
