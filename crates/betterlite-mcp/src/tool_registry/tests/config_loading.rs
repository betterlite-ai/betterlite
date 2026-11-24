use std::path::{Path, PathBuf};

use rstest::rstest;
use tracing::Level;

use crate::tool_registry::registry::{ToolRegistry, ToolRegistryError};

use super::support::{RecordingLoader, capture_logs, entries, names, object};

#[test]
fn a_missing_config_names_the_key() {
    let registry = ToolRegistry::default();
    let loader = RecordingLoader::knowing(&[]);
    let Err(error) = registry.load_tools_from_config(None, None, &loader) else {
        panic!("expected the missing config error");
    };
    assert!(matches!(&error, ToolRegistryError::Missing));
    assert_eq!(
        error.to_string(),
        "mcp_tools_config is required, please set `mcp_tools` in your proxy config"
    );
}

#[test]
fn a_non_mapping_entry_names_the_shape_and_keeps_earlier_tools() {
    let registry = ToolRegistry::default();
    let configured =
        entries("mcp_tools:\n  - {name: first, description: d, handler: module.one}\n  - nope\n");
    let loader = RecordingLoader::knowing(&["module.one"]);
    let Err(error) = registry.load_tools_from_config(Some(&configured), None, &loader) else {
        panic!("expected the non-mapping error");
    };
    assert!(matches!(&error, ToolRegistryError::NotAMapping));
    assert_eq!(
        error.to_string(),
        "mcp_tools_config must be a list of dictionaries"
    );
    assert_eq!(names(&registry), ["first"]);
}

#[rstest]
#[case::missing_name("mcp_tools:\n  - {description: d, handler: module.one}\n")]
#[case::null_name("mcp_tools:\n  - {name: null, description: d, handler: module.one}\n")]
#[case::empty_name("mcp_tools:\n  - {name: '', description: d, handler: module.one}\n")]
#[case::missing_description("mcp_tools:\n  - {name: first, handler: module.one}\n")]
#[case::null_description("mcp_tools:\n  - {name: first, description: null, handler: module.one}\n")]
#[case::empty_description("mcp_tools:\n  - {name: first, description: '', handler: module.one}\n")]
#[case::missing_handler("mcp_tools:\n  - {name: first, description: d}\n")]
#[case::null_handler("mcp_tools:\n  - {name: first, description: d, handler: null}\n")]
#[case::empty_handler("mcp_tools:\n  - {name: first, description: d, handler: ''}\n")]
fn skips_incomplete_entries(#[case] yaml: &str) {
    let registry = ToolRegistry::default();
    let configured = entries(yaml);
    let loader = RecordingLoader::knowing(&["module.one"]);
    registry
        .load_tools_from_config(Some(&configured), None, &loader)
        .unwrap();
    assert!(registry.list_tools(None).is_empty());
    assert!(loader.requests().is_empty());
}

#[test]
fn a_space_description_registers() {
    let registry = ToolRegistry::default();
    let configured =
        entries("mcp_tools:\n  - {name: first, description: ' ', handler: module.one}\n");
    let loader = RecordingLoader::knowing(&["module.one"]);
    registry
        .load_tools_from_config(Some(&configured), None, &loader)
        .unwrap();
    assert_eq!(names(&registry), ["first"]);
    assert_eq!(registry.get_tool("first").unwrap().description, " ");
}

#[rstest]
#[case::absent("mcp_tools:\n  - {name: first, description: d, handler: module.one}\n")]
#[case::null(
    "mcp_tools:\n  - {name: first, description: d, handler: module.one, input_schema: null}\n"
)]
fn defaults_the_input_schema_to_empty(#[case] yaml: &str) {
    let registry = ToolRegistry::default();
    let configured = entries(yaml);
    let loader = RecordingLoader::knowing(&["module.one"]);
    registry
        .load_tools_from_config(Some(&configured), None, &loader)
        .unwrap();
    let tool = registry.get_tool("first").unwrap();
    assert!(tool.input_schema.is_empty());
}

#[test]
fn keeps_the_configured_input_schema() {
    let registry = ToolRegistry::default();
    let configured = entries(
        "mcp_tools:\n  - name: first\n    description: d\n    handler: module.one\n    input_schema:\n      type: object\n      properties:\n        city: {type: string}\n",
    );
    let loader = RecordingLoader::knowing(&["module.one"]);
    registry
        .load_tools_from_config(Some(&configured), None, &loader)
        .unwrap();
    let tool = registry.get_tool("first").unwrap();
    assert_eq!(
        tool.input_schema,
        object(r#"{"type": "object", "properties": {"city": {"type": "string"}}}"#)
    );
}

#[test]
fn passes_the_handler_and_the_config_path() {
    let registry = ToolRegistry::default();
    let configured =
        entries("mcp_tools:\n  - {name: first, description: d, handler: module.one}\n");
    let loader = RecordingLoader::knowing(&["module.one"]);
    let path = Path::new("/etc/litellm/config.yaml");
    registry
        .load_tools_from_config(Some(&configured), Some(path), &loader)
        .unwrap();
    assert_eq!(
        loader.requests(),
        [("module.one".to_owned(), Some(PathBuf::from(path)))]
    );
}

#[test]
fn warns_and_skips_a_handler_the_loader_cannot_find() {
    let registry = ToolRegistry::default();
    let configured =
        entries("mcp_tools:\n  - {name: first, description: d, handler: module.missing}\n");
    let loader = RecordingLoader::knowing(&[]);
    let (logs, _guard) = capture_logs();
    registry
        .load_tools_from_config(Some(&configured), None, &loader)
        .unwrap();
    assert!(registry.list_tools(None).is_empty());
    assert_eq!(
        logs.messages_at(Level::WARN),
        ["Warning: Could not find handler module.missing for tool first"]
    );
}

#[test]
fn a_loader_error_stops_loading_and_keeps_earlier_tools() {
    let registry = ToolRegistry::default();
    let configured = entries(
        "mcp_tools:\n  - {name: first, description: d, handler: module.one}\n  - {name: second, description: d, handler: module.broken}\n  - {name: third, description: d, handler: module.one}\n",
    );
    let loader = RecordingLoader::failing_on(&["module.one"], "module.broken");
    let Err(error) = registry.load_tools_from_config(Some(&configured), None, &loader) else {
        panic!("expected the loader failure");
    };
    assert!(matches!(&error, ToolRegistryError::Handler(_)));
    assert_eq!(
        error.to_string(),
        "the handler \"module.broken\" could not be loaded"
    );
    assert_eq!(names(&registry), ["first"]);
    assert_eq!(loader.requests().len(), 2);
}

#[test]
fn logs_the_registered_names_after_loading() {
    let registry = ToolRegistry::default();
    let configured = entries(
        "mcp_tools:\n  - {name: first, description: d, handler: module.one}\n  - {name: second, description: d, handler: module.one}\n",
    );
    let loader = RecordingLoader::knowing(&["module.one"]);
    let (logs, _guard) = capture_logs();
    registry
        .load_tools_from_config(Some(&configured), None, &loader)
        .unwrap();
    assert_eq!(
        logs.messages_at(Level::DEBUG),
        [
            "Registered tool: first",
            "Registered tool: second",
            "all registered tools: [\"first\", \"second\"]"
        ]
    );
}
