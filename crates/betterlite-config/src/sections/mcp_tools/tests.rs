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

fn load(source: &str, lookup: &[(&str, &str)]) -> Result<McpToolsSection, ConfigError> {
    let config = ConfigLoader {
        lookup: populated(lookup),
    }
    .load(source)?;
    Ok(config.sections.mcp_tools)
}

fn populated(lookup: &[(&str, &str)]) -> BTreeMap<String, String> {
    lookup
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
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
    assert!(load(source, &[]).unwrap().entries.is_empty());
}

#[test]
fn reads_a_tool_entry() {
    let loaded = load(
        "mcp_tools:\n  - name: my_tool\n    description: A tool\n    input_schema:\n      type: object\n      properties:\n        x: {type: string}\n    handler: module.fn\n",
        &[],
    )
    .unwrap();
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
fn resolves_environment_references() {
    let loaded = load(
        "mcp_tools:\n  - name: os.environ/MCP_TOOL\n    description: os.environ/MCP_DESCRIPTION\n    handler: os.environ/MCP_HANDLER\n    input_schema:\n      type: object\n      properties:\n        city:\n          type: os.environ/MCP_TYPE\n",
        &[
            ("MCP_TOOL", "weather"),
            ("MCP_DESCRIPTION", "Weather tool"),
            ("MCP_HANDLER", "module.weather"),
            ("MCP_TYPE", "string"),
        ],
    )
    .unwrap();
    let tool = only_tool(&loaded);
    assert_eq!(tool.name.as_deref(), Some("weather"));
    assert_eq!(tool.description.as_deref(), Some("Weather tool"));
    assert_eq!(tool.handler.as_deref(), Some("module.weather"));
    let Some(Json::Object(properties)) = tool.input_schema.get("properties") else {
        panic!("expected a properties mapping");
    };
    let Some(Json::Object(city)) = properties.get("city") else {
        panic!("expected a city mapping");
    };
    assert_eq!(city.get("type"), Some(&Json::Text("string".to_owned())));
}

#[test]
fn reports_an_unset_environment_reference() {
    let ConfigError::MissingEnvironment { variable, path } = load(
        "mcp_tools:\n  - name: os.environ/MCP_TOOL\n    description: d\n    handler: h\n",
        &[],
    )
    .unwrap_err() else {
        panic!("expected a missing environment error");
    };
    assert_eq!(variable, "MCP_TOOL");
    assert_eq!(path, "mcp_tools[0].name");
}

#[test]
fn reports_an_unset_environment_reference_inside_the_schema() {
    let ConfigError::MissingEnvironment { variable, path } = load(
        "mcp_tools:\n  - name: first\n    description: d\n    handler: h\n    input_schema:\n      properties:\n        city:\n          type: os.environ/MCP_TYPE\n",
        &[],
    )
    .unwrap_err()
    else {
        panic!("expected a missing environment error");
    };
    assert_eq!(variable, "MCP_TYPE");
    assert_eq!(path, "mcp_tools[0].input_schema.properties.city.type");
}

#[test]
fn leaves_scalar_list_items_unresolved() {
    let loaded = load(
        "mcp_tools:\n  - name: first\n    description: d\n    handler: h\n    input_schema:\n      anyOf:\n        - {type: os.environ/MCP_TYPE}\n        - plain\n      enum: [os.environ/LEFT_ALONE]\n",
        &[("MCP_TYPE", "string")],
    )
    .unwrap();
    let tool = only_tool(&loaded);
    let Some(Json::List(items)) = tool.input_schema.get("anyOf") else {
        panic!("expected an anyOf list");
    };
    assert_eq!(
        items.first(),
        Some(&Json::Object(IndexMap::from([(
            "type".to_owned(),
            Json::Text("string".to_owned())
        )])))
    );
    assert_eq!(items.get(1), Some(&Json::Text("plain".to_owned())));
    let Some(Json::List(values)) = tool.input_schema.get("enum") else {
        panic!("expected an enum list");
    };
    assert_eq!(
        values.first(),
        Some(&Json::Text("os.environ/LEFT_ALONE".to_owned()))
    );
}

#[test]
fn keeps_non_mapping_entries() {
    let loaded = load(
        "mcp_tools:\n  - name: first\n    description: d\n    handler: h\n  - just a string\n  - 5\n  - null\n  - [a]\n",
        &[],
    )
    .unwrap();
    let kinds: Vec<bool> = loaded
        .entries
        .iter()
        .map(|entry| matches!(entry, McpToolEntry::Tool(_)))
        .collect();
    assert_eq!(kinds, [true, false, false, false, false]);
}

#[rstest]
#[case::absent("mcp_tools:\n  - {name: first, description: d, handler: h}\n", &[])]
#[case::null("mcp_tools:\n  - {name: first, description: d, handler: h, input_schema: null}\n", &[])]
#[case::object("mcp_tools:\n  - name: first\n    description: d\n    handler: h\n    input_schema:\n      type: object\n", &["type"])]
fn reads_the_input_schema(#[case] source: &str, #[case] keys: &[&str]) {
    let loaded = load(source, &[]).unwrap();
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
    let loaded = load(
        "mcp_tools:\n  - name: first\n    description: d\n    handler: h\n    enabled: true\n    extra: {a: 1}\n",
        &[],
    )
    .unwrap();
    let tool = only_tool(&loaded);
    assert_eq!(tool.name.as_deref(), Some("first"));
}

#[rstest]
#[case::mapping("mcp_tools:\n  name: first\n", false)]
#[case::text("mcp_tools: hello\n", false)]
#[case::number("mcp_tools: 5\n", false)]
#[case::boolean("mcp_tools: true\n", true)]
fn rejects_a_top_level_that_is_not_a_list(#[case] source: &str, #[case] value: bool) {
    let ConfigError::InvalidScalar { path, value: found } = load(source, &[]).unwrap_err() else {
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
    let ConfigError::InvalidScalar { path: found, .. } = load(source, &[]).unwrap_err() else {
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
