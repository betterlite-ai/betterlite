#![cfg(test)]

use betterlite_websearch::tools::{Tool, is_web_search_tool};

#[test]
fn web_search_tool() {
    let tool = Tool {
        tool_type: Some("web_search_20250305".to_owned()),
        name: Some("web_search".to_owned()),
        function: None,
        input_schema: None,
    };
    assert!(is_web_search_tool(&tool));
}
