use rstest::rstest;

use crate::tools::error::ToolFormatError;
use crate::tools::schema::MAX_SCHEMA_DEPTH;
use crate::tools::{ToolLimits, format_function_definitions, parse_tool_definitions};

use super::format;

const PAD_ENTRIES: usize = 7_500_000;

fn padded_leaf() -> String {
    let mut values = String::with_capacity(PAD_ENTRIES.saturating_mul(2));
    for index in 0..PAD_ENTRIES {
        if index > 0 {
            values.push(',');
        }
        values.push('0');
    }
    format!(r#"{{"type":"boolean","enum":[{values}]}}"#)
}

fn array_chain(levels: usize, leaf: &str) -> String {
    let mut inner = leaf.to_owned();
    for _ in 0..levels {
        inner = format!(r#"{{"type":"array","items":{inner}}}"#);
    }
    inner
}

fn object_chain(levels: usize, root_extra: &str, leaf: &str) -> String {
    let mut inner = leaf.to_owned();
    for _ in 0..levels {
        inner = format!(r#"{{"type":"object","properties":{{"p":{inner}}}}}"#);
    }
    format!(r#"{{"type":"object"{root_extra},"properties":{{"deep":{inner}}}}}"#)
}

fn tool_with(parameters: &str) -> String {
    format!(r#"[{{"type":"function","function":{{"name":"deep","parameters":{parameters}}}}}]"#)
}

fn formatted_tool(parameters: &str) -> String {
    let tools = tool_with(parameters);
    let definitions = parse_tool_definitions(tools.as_bytes(), ToolLimits::default())
        .expect("the tool fits the default limits");
    format_function_definitions(&definitions).expect("a schema under the depth cap formats")
}

#[rstest]
fn token_formatting_behavior_tests_behavior() {
    let leaf = padded_leaf();
    checks_object_chains(&leaf);
    checks_schema_depth();
}

fn checks_object_chains(leaf: &str) {
    let formatted = formatted_tool(&object_chain(60, "", leaf));
    assert!(formatted.contains("p?: boolean,"));

    let with_items = formatted_tool(&object_chain(60, r#","items":5"#, leaf));
    assert!(with_items.contains("p?: boolean,"));

    let too_deep = tool_with(&object_chain(70, "", leaf));
    let error = format(&too_deep).expect_err("70 object levels pass the depth cap");
    assert!(matches!(
        error,
        ToolFormatError::TooDeep {
            limit: MAX_SCHEMA_DEPTH
        }
    ));
}

fn checks_schema_depth() {
    let deepest = formatted_tool(&array_chain(MAX_SCHEMA_DEPTH, r#"{"type":"string"}"#));
    assert!(deepest.contains("type deep = () => any;"));

    let boundary = format!(
        r#"{{"type":"object","properties":{{"deep":{}}}}}"#,
        array_chain(MAX_SCHEMA_DEPTH.saturating_sub(1), r#"{"type":"string"}"#)
    );
    let rendered = formatted_tool(&boundary);
    assert!(rendered.contains("deep?: string[]"));

    let too_many_arrays = tool_with(&array_chain(
        MAX_SCHEMA_DEPTH.saturating_add(8),
        r#"{"type":"string"}"#,
    ));
    let too_deep_array =
        format(&too_many_arrays).expect_err("array levels past the depth cap are rejected");
    assert!(matches!(
        too_deep_array,
        ToolFormatError::TooDeep {
            limit: MAX_SCHEMA_DEPTH
        }
    ));
}
