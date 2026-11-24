#![cfg(test)]

use std::collections::BTreeMap;
use std::fs::{read, read_dir};
use std::io::Error;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::mcp_tools::{
    McpToolEntry, McpToolSettings, McpToolsError, McpToolsSection, decode_mcp_tools,
};

fn section(source: &str) -> McpToolsSection {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap()
    .sections
    .mcp_tools
}

fn load_error(source: &str) -> ConfigError {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .err()
    .unwrap()
}

fn only_tool(loaded: &McpToolsSection) -> &McpToolSettings {
    let [McpToolEntry::Tool(tool)] = loaded.entries.as_slice() else {
        panic!("expected one tool entry, got {:?}", loaded.entries);
    };
    tool
}

#[rstest]
#[case::absent("{}")]
#[case::null("mcp_tools: null\n")]
#[case::empty_list("mcp_tools: []\n")]
fn absent_or_empty_gives_no_entries(#[case] source: &str) {
    assert!(section(source).entries.is_empty());
}

#[test]
fn reads_a_tool_entry() {
    let loaded = section(
        "mcp_tools:\n  - name: my_tool\n    description: A tool\n    input_schema:\n      type: object\n      properties:\n        x: {type: string}\n    handler: module.fn\n",
    );
    let tool = only_tool(&loaded);
    assert_eq!(tool.name.as_deref(), Some("my_tool"));
    assert_eq!(tool.description.as_deref(), Some("A tool"));
    assert_eq!(tool.handler.as_deref(), Some("module.fn"));
    assert_eq!(
        tool.input_schema
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["type", "properties"]
    );
    assert_eq!(
        tool.input_schema.get("type"),
        Some(&Json::Text("object".to_owned()))
    );
    let Some(Json::Object(properties)) = tool.input_schema.get("properties") else {
        panic!("expected a properties mapping");
    };
    assert_eq!(
        properties.get("x"),
        Some(&Json::Object(IndexMap::from([(
            "type".to_owned(),
            Json::Text("string".to_owned())
        )])))
    );
}

#[test]
fn keeps_non_mapping_entries() {
    let loaded = section(
        "mcp_tools:\n  - name: first\n    description: d\n    handler: h\n  - just a string\n  - 5\n  - null\n  - [a]\n",
    );
    let kinds: Vec<bool> = loaded
        .entries
        .iter()
        .map(|entry| matches!(entry, McpToolEntry::Tool(_)))
        .collect();
    assert_eq!(kinds, [true, false, false, false, false]);
}

#[rstest]
#[case::absent("mcp_tools:\n  - {name: first, description: d, handler: h}\n", &[] as &[&str])]
#[case::null("mcp_tools:\n  - {name: first, description: d, handler: h, input_schema: null}\n", &[])]
#[case::object("mcp_tools:\n  - name: first\n    description: d\n    handler: h\n    input_schema:\n      type: object\n", &["type"])]
fn reads_the_input_schema(#[case] source: &str, #[case] keys: &[&str]) {
    let loaded = section(source);
    let tool = only_tool(&loaded);
    assert_eq!(
        tool.input_schema
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        keys
    );
}

#[test]
fn ignores_unknown_entry_keys() {
    let loaded = section(
        "mcp_tools:\n  - name: first\n    description: d\n    handler: h\n    enabled: true\n    extra: {a: 1}\n",
    );
    let tool = only_tool(&loaded);
    assert_eq!(tool.name.as_deref(), Some("first"));
}

#[rstest]
#[case::mapping("mcp_tools:\n  name: first\n", false)]
#[case::text("mcp_tools: hello\n", false)]
#[case::number("mcp_tools: 5\n", false)]
#[case::boolean("mcp_tools: true\n", true)]
fn rejects_a_top_level_that_is_not_a_list(#[case] source: &str, #[case] value: bool) {
    let ConfigError::InvalidScalar { path, value: found } = load_error(source) else {
        panic!("expected an invalid scalar for mcp_tools");
    };
    assert_eq!(path, "mcp_tools");
    assert_eq!(found, value);
}

#[rstest]
#[case::name("mcp_tools:\n  - name: 5\n", "mcp_tools[0].name")]
#[case::description(
    "mcp_tools:\n  - name: first\n    description: [a]\n",
    "mcp_tools[0].description"
)]
#[case::handler(
    "mcp_tools:\n  - name: first\n    description: d\n    handler: true\n",
    "mcp_tools[0].handler"
)]
#[case::input_schema(
    "mcp_tools:\n  - name: first\n    input_schema: hello\n",
    "mcp_tools[0].input_schema"
)]
#[case::second_entry("mcp_tools:\n  - name: first\n  - name: [x]\n", "mcp_tools[1].name")]
fn rejects_a_wrong_field_type(#[case] source: &str, #[case] path: &str) {
    let ConfigError::InvalidScalar { path: found, .. } = load_error(source) else {
        panic!("expected an invalid scalar naming the field");
    };
    assert_eq!(found, path);
}

#[test]
fn decodes_the_entries() {
    let loaded = decode_mcp_tools(
        b"mcp_tools:\n  - name: first\n    description: d\n    handler: h\n  - nope\n",
    )
    .unwrap();
    assert_eq!(loaded.entries.len(), 2);
    assert_eq!(loaded.entries[1], McpToolEntry::NotAMapping);
}

#[test]
fn decode_reports_yaml_and_config_errors() {
    assert!(matches!(
        decode_mcp_tools(b"mcp_tools: [\n").unwrap_err(),
        McpToolsError::Yaml { .. }
    ));
    assert!(matches!(
        decode_mcp_tools(b"mcp_tools: 5\n").unwrap_err(),
        McpToolsError::Config(ConfigError::InvalidScalar { .. })
    ));
}

#[test]
fn replays_the_mcp_tools_corpus() {
    read_dir("fuzz/corpus/mcp_tools")
        .unwrap()
        .try_for_each(|entry_result| {
            let entry = entry_result?;
            let bytes = read(entry.path())?;
            let _section = decode_mcp_tools(&bytes);
            Ok::<(), Error>(())
        })
        .unwrap();
}
