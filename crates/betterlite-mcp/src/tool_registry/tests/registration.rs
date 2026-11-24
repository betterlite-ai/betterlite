use tracing::Level;

use crate::tool_registry::registry::ToolRegistry;

use super::support::{capture_logs, names, object, silent};

#[test]
fn registers_a_tool_that_get_tool_returns() {
    let registry = ToolRegistry::default();
    registry.register_tool(
        "first",
        "First tool",
        object(r#"{"type": "object"}"#),
        silent(),
    );
    let tool = registry.get_tool("first").unwrap();
    assert_eq!(tool.name, "first");
    assert_eq!(tool.description, "First tool");
    assert_eq!(tool.input_schema, object(r#"{"type": "object"}"#));
}

#[test]
fn get_tool_returns_none_for_an_unknown_name() {
    let registry = ToolRegistry::default();
    assert!(registry.get_tool("missing").is_none());
}

#[test]
fn replacing_a_tool_keeps_its_first_position() {
    let registry = ToolRegistry::default();
    registry.register_tool("first", "Old", object("{}"), silent());
    registry.register_tool("second", "Second", object("{}"), silent());
    registry.register_tool("first", "New", object(r#"{"type": "object"}"#), silent());
    assert_eq!(names(&registry), ["first", "second"]);
    let tool = registry.get_tool("first").unwrap();
    assert_eq!(tool.description, "New");
    assert_eq!(tool.input_schema, object(r#"{"type": "object"}"#));
}

#[test]
fn logs_each_registration_at_debug() {
    let (logs, _guard) = capture_logs();
    let registry = ToolRegistry::default();
    registry.register_tool("first", "d", object("{}"), silent());
    registry.register_tool("second", "d", object("{}"), silent());
    assert_eq!(
        logs.messages_at(Level::DEBUG),
        ["Registered tool: first", "Registered tool: second"]
    );
}
