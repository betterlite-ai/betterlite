use betterlite_mcp::openapi_tools::output::OpenApiToolOutput;
use betterlite_mcp::tool_registry::registry::ToolRegistryError;
use rstest::rstest;

use super::support::{call, entries, fixture, tools_yaml};
use crate::app::mcp::python_tools::PythonToolsError;

#[tokio::test]
async fn loads_each_configured_handler_in_entry_order() {
    let fixture = fixture();
    let first = fixture
        .workspace
        .module("b8871_first", "def run():\n    return \"first\"\n");
    let second = fixture
        .workspace
        .module("b8871_second", "def run():\n    return \"second\"\n");
    let first_handler = format!("{first}.run");
    let second_handler = format!("{second}.run");
    let configured = entries(&tools_yaml(&[
        ("first", first_handler.as_str()),
        ("second", second_handler.as_str()),
    ]));
    fixture.load(Some(&configured)).await.unwrap();
    assert_eq!(fixture.names(), ["first", "second"]);
    assert_eq!(
        call(&fixture, "first", "{}").await.unwrap(),
        OpenApiToolOutput::Text("first".to_owned())
    );
    assert_eq!(
        call(&fixture, "second", "{}").await.unwrap(),
        OpenApiToolOutput::Text("second".to_owned())
    );
}

#[tokio::test]
async fn imports_a_module_below_the_config_folder() {
    let fixture = fixture();
    let module = fixture.workspace.nested_module(
        "helpers",
        "b8871_nested",
        "def echo(city):\n    return city\n",
    );
    let handler = format!("{module}.echo");
    let configured = entries(&tools_yaml(&[("echo", handler.as_str())]));
    fixture.load(Some(&configured)).await.unwrap();
    assert_eq!(
        call(&fixture, "echo", r#"{"city": "Paris"}"#)
            .await
            .unwrap(),
        OpenApiToolOutput::Text("Paris".to_owned())
    );
}

#[tokio::test]
async fn a_duplicate_name_replaces_the_handler_without_reordering() {
    let fixture = fixture();
    let first = fixture
        .workspace
        .module("b8871_dup_one", "def run():\n    return \"one\"\n");
    let second = fixture
        .workspace
        .module("b8871_dup_two", "def run():\n    return \"two\"\n");
    let third = fixture
        .workspace
        .module("b8871_dup_three", "def run():\n    return \"three\"\n");
    let first_handler = format!("{first}.run");
    let second_handler = format!("{second}.run");
    let third_handler = format!("{third}.run");
    let configured = entries(&tools_yaml(&[
        ("alpha", first_handler.as_str()),
        ("beta", second_handler.as_str()),
        ("alpha", third_handler.as_str()),
    ]));
    fixture.load(Some(&configured)).await.unwrap();
    assert_eq!(fixture.names(), ["alpha", "beta"]);
    assert_eq!(
        call(&fixture, "alpha", "{}").await.unwrap(),
        OpenApiToolOutput::Text("three".to_owned())
    );
}

#[rstest]
#[case::missing_name("mcp_tools:\n  - {description: d, handler: b8871_ghost.missing}\n")]
#[case::null_name("mcp_tools:\n  - {name: null, description: d, handler: b8871_ghost.missing}\n")]
#[case::empty_name("mcp_tools:\n  - {name: '', description: d, handler: b8871_ghost.missing}\n")]
#[case::missing_description("mcp_tools:\n  - {name: skipped, handler: b8871_ghost.missing}\n")]
#[case::null_description(
    "mcp_tools:\n  - {name: skipped, description: null, handler: b8871_ghost.missing}\n"
)]
#[case::empty_description(
    "mcp_tools:\n  - {name: skipped, description: '', handler: b8871_ghost.missing}\n"
)]
#[case::missing_handler("mcp_tools:\n  - {name: skipped, description: d}\n")]
#[case::empty_handler("mcp_tools:\n  - {name: skipped, description: d, handler: ''}\n")]
#[tokio::test]
async fn skips_an_incomplete_entry_without_importing_it(#[case] yaml: &str) {
    let fixture = fixture();
    let configured = entries(yaml);
    fixture.load(Some(&configured)).await.unwrap();
    assert!(fixture.names().is_empty());
}

#[tokio::test]
async fn skips_a_handler_that_resolved_to_none() {
    let fixture = fixture();
    let module = fixture.workspace.module("b8871_none", "handler = None\n");
    let handler = format!("{module}.handler");
    let configured = entries(&tools_yaml(&[("skipped", handler.as_str())]));
    fixture.load(Some(&configured)).await.unwrap();
    assert!(fixture.names().is_empty());
}

#[tokio::test]
async fn keeps_earlier_tools_when_a_later_import_fails() {
    let fixture = fixture();
    let first = fixture
        .workspace
        .module("b8871_kept", "def run():\n    return \"kept\"\n");
    let later = fixture
        .workspace
        .module("b8871_later", "def run():\n    return \"later\"\n");
    let first_handler = format!("{first}.run");
    let later_handler = format!("{later}.run");
    let configured = entries(&tools_yaml(&[
        ("kept", first_handler.as_str()),
        ("broken", "b8871_ghost.missing"),
        ("later", later_handler.as_str()),
    ]));
    let error = fixture.load(Some(&configured)).await.unwrap_err();
    match error {
        PythonToolsError::Load { handler, .. } => assert_eq!(handler, "b8871_ghost.missing"),
        PythonToolsError::Registry(other) => panic!("expected the import failure, got {other:?}"),
    }
    assert_eq!(fixture.names(), ["kept"]);
}

#[tokio::test]
async fn keeps_earlier_tools_when_a_later_entry_is_not_a_mapping() {
    let fixture = fixture();
    let first = fixture
        .workspace
        .module("b8871_map", "def run():\n    return \"kept\"\n");
    let first_handler = format!("{first}.run");
    let kept = tools_yaml(&[("kept", first_handler.as_str())]);
    let configured = entries(&format!("{kept}  - not-a-mapping\n"));
    let error = fixture.load(Some(&configured)).await.unwrap_err();
    assert!(matches!(
        error,
        PythonToolsError::Registry(ToolRegistryError::NotAMapping)
    ));
    assert_eq!(fixture.names(), ["kept"]);
}

#[tokio::test]
async fn a_missing_list_is_the_registry_missing_error() {
    let fixture = fixture();
    let error = fixture.load(None).await.unwrap_err();
    match error {
        PythonToolsError::Registry(ToolRegistryError::Missing) => {}
        PythonToolsError::Registry(other) => {
            panic!("expected the missing config error, got {other:?}")
        }
        PythonToolsError::Load { handler, .. } => {
            panic!("expected the missing config error, got the load of {handler}")
        }
    }
}

#[tokio::test]
async fn an_empty_list_registers_nothing() {
    let fixture = fixture();
    let configured = entries("mcp_tools: []\n");
    fixture.load(Some(&configured)).await.unwrap();
    assert!(fixture.names().is_empty());
}

#[tokio::test]
async fn loading_never_calls_the_handler() {
    let fixture = fixture();
    let marker = fixture.workspace.directory().join("b8871-called.txt");
    let path = marker.to_str().unwrap();
    let source =
        format!("def run():\n    open(\"{path}\", \"w\").write(\"called\")\n    return \"idle\"\n");
    let module = fixture.workspace.module("b8871_not_called", &source);
    let handler = format!("{module}.run");
    let configured = entries(&tools_yaml(&[("idle", handler.as_str())]));
    fixture.load(Some(&configured)).await.unwrap();
    assert_eq!(fixture.names(), ["idle"]);
    assert!(!marker.exists());
    assert_eq!(
        call(&fixture, "idle", "{}").await.unwrap(),
        OpenApiToolOutput::Text("idle".to_owned())
    );
    assert!(marker.exists());
}
