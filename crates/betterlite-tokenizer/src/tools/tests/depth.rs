use rstest::rstest;

use crate::tools::error::ToolFormatError;
use crate::tools::schema::MAX_SCHEMA_DEPTH;
use crate::tools::{ToolLimits, format_function_definitions, parse_tool_definitions};

use super::format;

fn nested_arrays(levels: usize, leaf: &str) -> String {
    (0..levels).fold(leaf.to_owned(), |inner, _| {
        format!(r#"{{"type":"array","items":{inner}}}"#)
    })
}

fn tool_with(schema: &str) -> String {
    let prefix = r#"[{"type":"function","function":{"name":"deep","parameters":{"type":"object","properties":{"deep":"#;
    let suffix = "}}}}}]";
    format!("{prefix}{schema}{suffix}")
}

fn leaf_with(properties: usize) -> String {
    let entries = (0..properties)
        .map(|index| format!(r#""f{index}":{{"type":"string"}}"#))
        .collect::<Vec<String>>()
        .join(",");
    let mut leaf = String::from(r#"{"type":"object","properties":{"#);
    leaf.push_str(&entries);
    leaf.push_str("}}");
    leaf
}

#[rstest]
fn token_formatting_behavior_tests_behavior() {
    let too_deep = nested_arrays(MAX_SCHEMA_DEPTH + 8, r#"{"type":"string"}"#);
    let error = format(&tool_with(&too_deep)).expect_err("a schema past the depth cap is rejected");
    assert!(matches!(
        error,
        ToolFormatError::TooDeep {
            limit: MAX_SCHEMA_DEPTH
        }
    ));

    let deep = tool_with(&nested_arrays(40, &leaf_with(20_000)));
    let limits = ToolLimits {
        max_bytes: deep.len(),
        max_tools: 8,
    };
    let definitions = parse_tool_definitions(deep.as_bytes(), limits)
        .expect("the definitions fit the byte limit");
    let formatted =
        format_function_definitions(&definitions).expect("a deep schema under the cap formats");
    assert!(formatted.contains("f19999?: string,"));

    // 60 array levels under the default 16 MiB limit: the scan stays linear in the bytes
    let near_limit = tool_with(&nested_arrays(60, &leaf_with(550_000)));
    assert!(near_limit.len() > 14 * 1024 * 1024);
    let defaults = ToolLimits::default();
    let large = parse_tool_definitions(near_limit.as_bytes(), defaults)
        .expect("the definitions fit the default byte limit");
    let rendered =
        format_function_definitions(&large).expect("a deep schema near the byte limit formats");
    assert!(rendered.contains("f549999?: string,"));
    assert!(rendered.ends_with("} // namespace functions"));
}
