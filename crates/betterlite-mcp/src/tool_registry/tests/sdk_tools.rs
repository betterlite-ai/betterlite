use crate::tool_registry::registry::ToolRegistry;

use super::support::{object, registry, silent};

#[test]
fn converts_in_the_given_order() {
    let loaded = registry(&["first", "second"]);
    let listed = loaded.list_tools(None);
    let converted = ToolRegistry::to_sdk_tools(&listed);
    let found: Vec<&str> = converted.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(found, ["first", "second"]);
}

#[test]
fn writes_the_mcp_wire_field_names() {
    let loaded = ToolRegistry::default();
    loaded.register_tool(
        "first",
        "First tool",
        object(r#"{"type": "object"}"#),
        silent(),
    );
    loaded.register_tool("second", "Second tool", object("{}"), silent());
    let listed = loaded.list_tools(None);
    let converted = ToolRegistry::to_sdk_tools(&listed);
    assert_eq!(
        serde_json::to_string(&converted[0]).unwrap(),
        r#"{"name":"first","description":"First tool","inputSchema":{"type":"object"}}"#
    );
    assert_eq!(
        serde_json::to_string(&converted[1]).unwrap(),
        r#"{"name":"second","description":"Second tool","inputSchema":{}}"#
    );
}
