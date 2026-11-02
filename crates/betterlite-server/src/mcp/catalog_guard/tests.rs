#![cfg(test)]

use std::collections::BTreeMap;

use super::description_overrides::apply_description_overrides;
use super::guard::{GuardrailCallContext, GuardrailFailure, GuardrailHook, ScanContext};
use super::headers::{HeaderSanitizeContext, logging_safe_mcp_headers};
use super::pin::pin_tool_catalog;
use super::prefix::{PrefixPolicy, strip_known_server_prefix};
use super::types::{MCPServer, MCPTool};
use super::{CatalogAlert, PinnedCatalogDrift, block_reason, masked_tool};
use betterlite_db::mcp_stored_contracts::{JsonValue, PinnedMCPTool};
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::sleep;

fn tool(name: &str, description: Option<&str>) -> MCPTool {
    MCPTool {
        name: name.to_owned(),
        description: description.map(str::to_owned),
        title: None,
        input_schema: BTreeMap::new(),
        output_schema: None,
        execution: None,
        annotations: None,
        icons: None,
        meta: None,
    }
}

fn pinned(description: &str) -> PinnedMCPTool {
    let source = format!(r#"{{"description":"{description}","input_schema":{{}}}}"#);
    serde_json::from_str(&source).expect("valid pinned tool")
}

fn server() -> MCPServer {
    MCPServer {
        server_id: "server-id".to_owned(),
        name: "server".to_owned(),
        alias: None,
        server_name: None,
        short_prefix: None,
        extra_headers: Vec::new(),
        tool_name_to_description: BTreeMap::new(),
    }
}

#[test]
fn catalog_alert_types() {
    let alert = CatalogAlert {
        signature: "a".to_owned(),
        message: "m".to_owned(),
    };
    assert_eq!(alert.signature, "a");
    assert_eq!(alert.message, "m");
}

#[test]
fn drift_alert_is_absent_when_empty() {
    let drift = PinnedCatalogDrift {
        added: Vec::new(),
        removed: Vec::new(),
        changed: Vec::new(),
    };
    assert_eq!(drift.alert(&server()), None);
}

#[test]
fn drift_alert_uses_pipe_signature_and_labels() {
    let drift = PinnedCatalogDrift {
        added: vec!["new".to_owned()],
        removed: vec!["old".to_owned()],
        changed: vec!["changed".to_owned()],
    };
    let alert = drift.alert(&server()).expect("non-empty drift alerts");
    assert_eq!(
        alert.signature,
        "added: `new`|removed: `old`|changed: `changed`"
    );
    assert!(alert.message.contains("serving the pinned tools"));
}

#[test]
fn pin_intersection_restores_changed_description() {
    let mut kept = tool("kept", Some("new"));
    kept.annotations = Some(BTreeMap::from([(
        "readOnlyHint".to_owned(),
        JsonValue::Bool(true),
    )]));
    kept.output_schema = Some(BTreeMap::from([(
        "type".to_owned(),
        JsonValue::String("object".to_owned()),
    )]));
    kept.execution = Some(BTreeMap::from([(
        "taskSupport".to_owned(),
        JsonValue::String("forbidden".to_owned()),
    )]));
    let upstream = vec![kept.clone(), tool("added", Some("x"))];
    let mut pins = BTreeMap::new();
    pins.insert("kept".to_owned(), pinned("old"));
    pins.insert("removed".to_owned(), pinned("gone"));
    let (served, drift) = pin_tool_catalog(&upstream, &pins);
    assert_eq!(served[0].description.as_deref(), Some("old"));
    assert_eq!(served[0].annotations, kept.annotations);
    assert_eq!(served[0].output_schema, kept.output_schema);
    assert_eq!(served[0].execution, kept.execution);
    let catalog_drift = drift.expect("catalog changes alert");
    assert_eq!(catalog_drift.added, vec!["added"]);
    assert_eq!(catalog_drift.removed, vec!["removed"]);
    assert_eq!(catalog_drift.changed, vec!["kept"]);
}

#[test]
fn override_uses_stripped_tool_name() {
    let mut configured = server();
    configured.alias = Some("team-server".to_owned());
    configured
        .tool_name_to_description
        .insert("search".to_owned(), "safe".to_owned());
    let output = apply_description_overrides(
        &[tool("team-server-search", Some("old"))],
        &configured,
        &PrefixPolicy::default(),
    );
    assert_eq!(output[0].description.as_deref(), Some("safe"));
}

#[test]
fn override_preserves_annotations_and_output_schema() {
    let mut configured = server();
    configured
        .tool_name_to_description
        .insert("search".to_owned(), "safe".to_owned());
    let mut original = tool("search", Some("old"));
    original.annotations = Some(BTreeMap::from([(
        "readOnlyHint".to_owned(),
        JsonValue::Bool(true),
    )]));
    original.output_schema = Some(BTreeMap::from([(
        "type".to_owned(),
        JsonValue::String("object".to_owned()),
    )]));
    original.execution = Some(BTreeMap::from([(
        "taskSupport".to_owned(),
        JsonValue::String("forbidden".to_owned()),
    )]));
    let result =
        apply_description_overrides(&[original.clone()], &configured, &PrefixPolicy::default());
    assert_eq!(result[0].description.as_deref(), Some("safe"));
    assert_eq!(result[0].annotations, original.annotations);
    assert_eq!(result[0].output_schema, original.output_schema);
    assert_eq!(result[0].execution, original.execution);
}

#[test]
fn strip_prefix_longest_match_and_fallbacks() {
    let mut configured = server();
    configured.server_id = "server-id".to_owned();
    configured.alias = Some("server".to_owned());
    let policy = PrefixPolicy::default();
    assert_eq!(
        strip_known_server_prefix("server-id-tool", Some(&configured), &policy),
        "tool"
    );
    assert_eq!(
        strip_known_server_prefix("bare-tool", Some(&configured), &policy),
        "bare-tool"
    );
    assert_eq!(strip_known_server_prefix("one-tool", None, &policy), "tool");
}

#[test]
fn pin_empty_catalog_has_no_drift() {
    let (served, drift) = pin_tool_catalog(&[], &BTreeMap::new());
    assert!(served.is_empty());
    assert!(drift.is_none());
}

#[test]
fn json_value_can_represent_schema() {
    let value = JsonValue::Object(BTreeMap::from([(
        String::from("type"),
        JsonValue::String(String::from("object")),
    )]));
    assert!(matches!(value, JsonValue::Object(_)));
}

#[test]
fn scannable_text_skips_empty_tool() {
    let empty = tool("empty", Some(""));
    assert!(!super::has_scannable_text(&empty));
    assert!(super::has_scannable_text(&tool("described", Some("text"))));
}

#[test]
fn mask_applies_guarded_description_and_schema() {
    let mut original = tool("search", Some("old"));
    original.annotations = Some(BTreeMap::from([(
        "readOnlyHint".to_owned(),
        JsonValue::Bool(true),
    )]));
    original.output_schema = Some(BTreeMap::from([(
        "type".to_owned(),
        JsonValue::String("object".to_owned()),
    )]));
    original.title = Some("Search title".to_owned());
    original.meta = Some(BTreeMap::from([(
        "source".to_owned(),
        JsonValue::String("upstream".to_owned()),
    )]));
    let schema = BTreeMap::from([("type".to_owned(), JsonValue::String("object".to_owned()))]);
    let guarded = BTreeMap::from([
        (
            "mcp_tool_description".to_owned(),
            JsonValue::String("safe".to_owned()),
        ),
        (
            "mcp_input_schema".to_owned(),
            JsonValue::Object(schema.clone()),
        ),
    ]);
    let masked = masked_tool(&original, &guarded).expect("valid guarded fields");
    assert_eq!(masked.description.as_deref(), Some("safe"));
    assert_eq!(masked.input_schema, schema);
    assert_eq!(masked.annotations, original.annotations);
    assert_eq!(masked.output_schema, original.output_schema);
    assert_eq!(masked.title, original.title);
    assert_eq!(masked.meta, original.meta);
    assert_eq!(
        masked_tool(&original, &BTreeMap::new()).expect("missing fields keep originals"),
        original
    );
}

#[test]
fn malformed_guarded_values_are_validation_failures() {
    let original = tool("search", Some("old"));
    let bad_description = BTreeMap::from([(
        "mcp_tool_description".to_owned(),
        JsonValue::Number(serde_json::Number::from(42_i64)),
    )]);
    let bad_schema =
        BTreeMap::from([("mcp_input_schema".to_owned(), JsonValue::Array(Vec::new()))]);
    assert_eq!(
        masked_tool(&original, &bad_description)
            .expect_err("invalid description is rejected")
            .exception_type,
        "ValidationError"
    );
    assert_eq!(
        masked_tool(&original, &bad_schema)
            .expect_err("invalid schema is rejected")
            .exception_type,
        "ValidationError"
    );
}

#[test]
fn block_reason_extracts_detail_then_falls_back() {
    let detail = GuardrailFailure {
        exception_type: "GuardError".to_owned(),
        exception_message: "ignored".to_owned(),
        detail_error: Some("blocked".to_owned()),
    };
    assert_eq!(block_reason(&detail), "blocked");
    let fallback = GuardrailFailure {
        exception_type: "GuardError".to_owned(),
        exception_message: String::new(),
        detail_error: None,
    };
    assert_eq!(block_reason(&fallback), "GuardError");
}

#[test]
fn header_sanitize_drops_proxy_credentials_and_host() {
    let raw = BTreeMap::from([
        ("Host".to_owned(), "deployment.internal".to_owned()),
        ("Authorization".to_owned(), "proxy-key".to_owned()),
        ("X-Api-Key".to_owned(), "upstream-key".to_owned()),
        ("X-Trace".to_owned(), "visible".to_owned()),
    ]);
    let context = HeaderSanitizeContext {
        upstream_credential_names: &["X-Api-Key".to_owned()],
        identity_header_names: &[],
        custom_key_header_name: Some("X-Deployment-Key"),
        client_side_auth_header_name: "Authorization",
    };
    let safe = logging_safe_mcp_headers(&raw, &context);
    assert_eq!(
        safe,
        BTreeMap::from([("x-trace".to_owned(), "visible".to_owned())])
    );
}

#[test]
fn header_sanitize_excludes_other_server_credentials_and_keeps_identity() {
    let raw = BTreeMap::from([
        ("X-Other-Server-Token".to_owned(), "secret".to_owned()),
        ("X-User-Id".to_owned(), "user-1".to_owned()),
        (
            "Authorization".to_owned(),
            "Bearer sk-ant-oat-test".to_owned(),
        ),
    ]);
    let context = HeaderSanitizeContext {
        upstream_credential_names: &["X-Other-Server-Token".to_owned(), "X-User-Id".to_owned()],
        identity_header_names: &["X-User-Id".to_owned()],
        custom_key_header_name: None,
        client_side_auth_header_name: "x-mcp-auth",
    };
    let safe = logging_safe_mcp_headers(&raw, &context);
    assert!(!safe.contains_key("x-other-server-token"));
    assert_eq!(safe.get("x-user-id").map(String::as_str), Some("user-1"));
    assert_eq!(
        safe.get("authorization").map(String::as_str),
        Some("***REDACTED***")
    );
}

struct ControlledHook {
    outcomes: BTreeMap<String, Result<Option<BTreeMap<String, JsonValue>>, GuardrailFailure>>,
    active: Arc<AtomicUsize>,
    maximum: Arc<AtomicUsize>,
    calls: Arc<Mutex<Vec<RecordedCall>>>,
}

type RecordedCall = (String, String, bool, BTreeMap<String, String>);

impl GuardrailHook for ControlledHook {
    fn pre_call_hook(
        &self,
        context: GuardrailCallContext<'_>,
    ) -> impl Future<Output = Result<Option<BTreeMap<String, JsonValue>>, GuardrailFailure>> + Send
    {
        let active = Arc::clone(&self.active);
        let maximum = Arc::clone(&self.maximum);
        let calls = Arc::clone(&self.calls);
        let outcome = self
            .outcomes
            .get(context.tool_name)
            .cloned()
            .unwrap_or(Ok(None));
        let record = (
            context.call_type.as_str().to_owned(),
            context.tool_name.to_owned(),
            context.guardrails_only,
            context.headers.clone(),
        );
        async move {
            let previous = active.fetch_add(1, Ordering::SeqCst);
            let current = previous.saturating_add(1);
            maximum.fetch_max(current, Ordering::SeqCst);
            calls.lock().expect("calls mutex available").push(record);
            sleep(Duration::from_millis(5)).await;
            active.fetch_sub(1, Ordering::SeqCst);
            outcome
        }
    }
}

fn scan_context<'context>(
    server: &'context MCPServer,
    headers: &'context BTreeMap<String, String>,
    credentials: &'context [String],
) -> ScanContext<'context> {
    ScanContext {
        server,
        caller: None,
        headers,
        identity_header_names: &[],
        custom_key_header_name: None,
        client_side_auth_header_name: "x-mcp-auth",
        upstream_credential_names: credentials,
    }
}

#[tokio::test]
async fn scanning_blocks_malformed_guarded_values_and_records_call_context() {
    let hook = ControlledHook {
        outcomes: BTreeMap::from([(
            "bad".to_owned(),
            Ok(Some(BTreeMap::from([(
                "mcp_tool_description".to_owned(),
                JsonValue::Number(serde_json::Number::from(42_i64)),
            )]))),
        )]),
        active: Arc::new(AtomicUsize::new(0)),
        maximum: Arc::new(AtomicUsize::new(0)),
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let headers = BTreeMap::from([
        ("X-Other-Server-Token".to_owned(), "secret".to_owned()),
        ("X-Trace".to_owned(), "trace".to_owned()),
    ]);
    let credentials = vec!["X-Other-Server-Token".to_owned()];
    let server = server();
    let context = scan_context(&server, &headers, &credentials);
    let tools = vec![tool("bad", Some("description")), tool("good", Some("text"))];
    let result = super::scan_tool_descriptions(&tools, &context, &hook).await;
    assert_eq!(result.served, vec![tool("good", Some("text"))]);
    assert_eq!(result.blocked[0].name, "bad");
    assert!(result.blocked[0].reason.contains("mcp_tool_description"));
    let calls = hook.calls.lock().expect("calls mutex available");
    assert!(
        calls
            .iter()
            .all(|(kind, _, only, _)| kind == "list_mcp_tools" && *only)
    );
    assert!(
        calls
            .iter()
            .all(|(_, _, _, safe)| !safe.contains_key("x-other-server-token"))
    );
    drop(calls);
}

#[tokio::test]
async fn scanning_runs_at_most_eight_tools_concurrently_in_order() {
    let hook = ControlledHook {
        outcomes: BTreeMap::new(),
        active: Arc::new(AtomicUsize::new(0)),
        maximum: Arc::new(AtomicUsize::new(0)),
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let headers = BTreeMap::new();
    let credentials = Vec::new();
    let server = server();
    let context = scan_context(&server, &headers, &credentials);
    let tools: Vec<MCPTool> = (0_usize..17_usize)
        .map(|index| tool(&format!("tool-{index}"), Some("text")))
        .collect();
    let result = super::scan_tool_descriptions(&tools, &context, &hook).await;
    assert_eq!(result.served, tools);
    assert_eq!(hook.maximum.load(Ordering::SeqCst), 8);
}
