use betterlite_mcp::tool_registry::handler::ToolHandlerError;
use betterlite_mcp::tool_registry::registry::ToolRegistry;
use betterlite_prompts::arguments::json::Object;
use betterlite_pyhost::instance_loader::LoadError;
use rstest::rstest;
use tokio::task::JoinSet;
use tokio::time::advance;

use super::support::{
    context, entries, fixture, load_tool, on_sys_path, tools_yaml, workspace, yield_times,
};
use crate::app::mcp::python_tools::{PythonToolsError, load_configured_tools};
use crate::python_hooks::HooksError;
use crate::python_hooks::PythonHooks;
use crate::python_hooks::deadline::HOOK_DEADLINE;

#[tokio::test]
async fn an_absent_list_never_starts_the_interpreter() {
    let fixture = fixture();
    assert!(fixture.load(None).await.is_err());
    assert!(!on_sys_path(fixture.workspace.directory()));
}

#[tokio::test]
async fn an_empty_list_never_starts_the_interpreter() {
    let fixture = fixture();
    fixture
        .load(Some(&entries("mcp_tools: []\n")))
        .await
        .unwrap();
    assert!(!on_sys_path(fixture.workspace.directory()));
}

#[tokio::test]
async fn a_skipped_entry_never_starts_the_interpreter() {
    let fixture = fixture();
    let configured = entries("mcp_tools:\n  - {name: skipped, handler: b8871_ghost.missing}\n");
    fixture.load(Some(&configured)).await.unwrap();
    assert!(!on_sys_path(fixture.workspace.directory()));
}

#[rstest]
#[case::s3("s3")]
#[case::gcs("gcs")]
#[tokio::test]
async fn a_remote_handler_needs_the_config_file_provenance(#[case] scheme: &str) {
    let workspace = workspace();
    let hooks = PythonHooks::new(None);
    let registry = ToolRegistry::default();
    let handler = format!("{scheme}://bucket/b8871_remote.handler");
    let configured = entries(&tools_yaml(&[("remote", handler.as_str())]));
    let error = load_configured_tools(
        &registry,
        &hooks,
        Some(&configured),
        Some(&workspace.config_path),
    )
    .await
    .unwrap_err();
    match error {
        PythonToolsError::Load {
            handler: failed,
            source,
        } => {
            assert_eq!(failed, handler);
            match *source {
                HooksError::Load {
                    source: load_error, ..
                } => assert!(matches!(*load_error, LoadError::RemoteNeedsConfigPath)),
                other @ (HooksError::Interpreter { .. }
                | HooksError::InterpreterTask { .. }
                | HooksError::InterpreterMissing
                | HooksError::CallbackIsClass { .. }
                | HooksError::CallbackNotDispatchable { .. }
                | HooksError::Validate { .. }
                | HooksError::Call(_)
                | HooksError::StartupSpec { .. }
                | HooksError::Startup { .. }) => {
                    panic!("expected the loader refusal, got {other:?}")
                }
            }
        }
        PythonToolsError::Registry(other) => panic!("expected the loader refusal, got {other:?}"),
    }
    assert!(registry.list_tools(None).is_empty());
}

#[tokio::test]
async fn a_local_module_needs_the_config_file_provenance() {
    let workspace = workspace();
    let module = workspace.module(
        "b8871_unprovenanced",
        "def echo():\n    return \"echoed\"\n",
    );
    let handler = format!("{module}.echo");
    let hooks = PythonHooks::new(None);
    let registry = ToolRegistry::default();
    let configured = entries(&tools_yaml(&[("echo", handler.as_str())]));
    let error = load_configured_tools(
        &registry,
        &hooks,
        Some(&configured),
        Some(&workspace.config_path),
    )
    .await
    .unwrap_err();
    match error {
        PythonToolsError::Load {
            handler: failed, ..
        } => assert_eq!(failed, handler),
        PythonToolsError::Registry(other) => panic!("expected the import failure, got {other:?}"),
    }
    assert!(registry.list_tools(None).is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_slow_handler_is_cut_off_at_the_hook_deadline() {
    let fixture = fixture();
    let source = "import time\n\ndef slow():\n    time.sleep(0.2)\n    return \"late\"\n";
    load_tool(&fixture, "b8871_slow", source, "slow").await;
    let handler = fixture.registry.get_tool("tool").unwrap().handler;
    let mut calls = JoinSet::new();
    calls.spawn(async move {
        let arguments = Object::new();
        handler.call(&arguments, &context()).await
    });
    yield_times(100).await;
    advance(HOOK_DEADLINE).await;
    match calls.join_next().await.unwrap().unwrap() {
        Err(ToolHandlerError::Failed(message)) => {
            assert_eq!(message, "the tool handler did not finish within 30s");
        }
        other => panic!("expected the deadline failure, got {other:?}"),
    }
}
