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

    let formatted = formatted_tool(&object_chain(60, "", &leaf));
    assert!(formatted.contains("p?: boolean,"));

    let with_items = formatted_tool(&object_chain(60, r#","items":5"#, &leaf));
    assert!(with_items.contains("p?: boolean,"));

    let too_deep = tool_with(&object_chain(70, "", &leaf));
    let error = format(&too_deep).expect_err("70 object levels pass the depth cap");
    assert!(matches!(
        error,
        ToolFormatError::TooDeep {
            limit: MAX_SCHEMA_DEPTH
        }
    ));
}
