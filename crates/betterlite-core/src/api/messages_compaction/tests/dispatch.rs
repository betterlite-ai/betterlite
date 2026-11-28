use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Object;

use crate::api::messages_compaction::dispatch::{DispatchRequest, apply_context_management};
use crate::api::messages_compaction::error::DispatchError;
use crate::api::messages_context_clear::constants::{CLEAR_TOOL_USES_EDIT_TYPE, COMPACT_EDIT_TYPE};
use crate::api::messages_context_clear::result::{AppliedEdit, PolyfillResult};
use crate::api::summary_execution::completion::SummaryCompletion;

use super::support::{
    NeverCalled, ScriptedEditor, ScriptedError, ScriptedResult, applied, block, capture_logs,
    decided, field, iteration, names, object_keys, parse, polyfilled, principal, request,
    text_field,
};

const BASE_SYSTEM: &str = r#""BASE SYSTEM""#;
const COMPACTED_SYSTEM: &str = r#""COMPACTED SYSTEM""#;
const METADATA: &str = r#"{"trace_id":"t1"}"#;
const TOOLS: &str = r#"[{"name":"lookup"}]"#;
const M0: &str = r#"[{"role":"user","content":"first"}]"#;
const M1: &str = r#"[{"role":"user","content":"after clear"}]"#;
const M2: &str = r#"[{"role":"user","content":"after compact"}]"#;
const M3: &str = r#"[{"role":"user","content":"after second clear"}]"#;
const CLEAR_KEYS: [&str; 6] = [
    "editor",
    "model",
    "messages",
    "tools",
    "system",
    "edit_spec",
];
const COMPACT_KEYS: [&str; 9] = [
    "editor",
    "model",
    "messages",
    "tools",
    "system",
    "edit_spec",
    "litellm_metadata",
    "llm_router",
    "user_api_key_auth",
];

fn three_compactions() -> Json {
    parse(
        r#"{"edits":[{"type":"compact_20260112"},{"type":"compact_20260112"},
            {"type":"compact_20260112"}]}"#,
    )
}

fn flow_spec() -> Json {
    parse(
        r#"{"edits":[{"type":"clear_tool_uses_20250919"},{"type":"compact_20260112"},
            {"type":"clear_tool_uses_20250919"}]}"#,
    )
}

fn flow_editor() -> ScriptedEditor {
    ScriptedEditor::new(
        vec![
            ScriptedResult::Legacy {
                messages: decided(M1),
                applied: None,
            },
            ScriptedResult::Legacy {
                messages: decided(M3),
                applied: Some(applied(2)),
            },
        ],
        vec![ScriptedResult::Polyfilled(polyfilled(
            decided(M2),
            Some(parse(COMPACTED_SYSTEM)),
            vec![applied(1)],
            Some(block("A")),
            Some(vec![iteration(10, 4)]),
        ))],
    )
}

fn flow_dispatch<'request>(
    spec: &'request Json,
    summary_metadata: &'request Object,
    router: &'request dyn SummaryCompletion,
    request_principal: &'request UserAPIKeyAuth,
) -> DispatchRequest<'request> {
    let mut dispatch = request(decided(M0), Some(parse(BASE_SYSTEM)), Some(spec));
    dispatch.litellm_metadata = Some(summary_metadata);
    dispatch.llm_router = Some(router);
    dispatch.user_api_key_auth = Some(request_principal);
    dispatch
}

fn metadata() -> Object {
    serde_json::from_str(METADATA).expect("the metadata decodes")
}

fn assert_flow_result(result: &PolyfillResult) {
    assert_eq!(result.messages, decided(M3));
    assert_eq!(result.system, Some(parse(COMPACTED_SYSTEM)));
    assert_eq!(result.applied_edits, vec![applied(1), applied(2)]);
    assert_eq!(result.compaction_block, Some(block("A")));
    assert_eq!(result.iterations_usage, Some(vec![iteration(10, 4)]));
}

fn assert_clear_call(call: &Json, messages: &str, system: &str) {
    assert_eq!(object_keys(call), names(&CLEAR_KEYS));
    assert_eq!(text_field(call, "editor"), Some(CLEAR_TOOL_USES_EDIT_TYPE));
    assert_eq!(text_field(call, "model"), Some("main-model"));
    assert_eq!(
        field(call, "messages"),
        Some(&Json::List(decided(messages)))
    );
    assert_eq!(field(call, "system"), Some(&parse(system)));
    assert_eq!(field(call, "tools"), Some(&parse(TOOLS)));
}

fn assert_compact_call(call: &Json) {
    assert_eq!(object_keys(call), names(&COMPACT_KEYS));
    assert_eq!(text_field(call, "editor"), Some(COMPACT_EDIT_TYPE));
    assert_eq!(field(call, "messages"), Some(&Json::List(decided(M1))));
    assert_eq!(field(call, "system"), Some(&parse(BASE_SYSTEM)));
    assert_eq!(field(call, "tools"), Some(&parse(TOOLS)));
    assert_eq!(field(call, "litellm_metadata"), Some(&parse(METADATA)));
    assert_eq!(text_field(call, "llm_router"), Some("router"));
    assert_eq!(text_field(call, "user_api_key_auth"), Some("present"));
}

#[tokio::test]
async fn unknown_edits_preserve_state() {
    let editor = ScriptedEditor::new(Vec::new(), Vec::new());
    let messages = decided(r#"[{"role":"user","content":"hello"}]"#);
    let system = parse(BASE_SYSTEM);
    let spec = parse(r#"{"edits":[{"type":"future"},{"type":7},{}]}"#);
    let (logs, guard) = capture_logs();
    let dispatch = request(messages.clone(), Some(system.clone()), Some(&spec));
    let result = apply_context_management(dispatch, &editor)
        .await
        .expect("an unknown edit type only logs");
    drop(guard);

    assert_eq!(result.messages, messages);
    assert_eq!(result.system, Some(system));
    assert!(result.applied_edits.is_empty());
    assert!(result.compaction_block.is_none());
    assert!(result.iterations_usage.is_none());
    assert!(editor.calls().is_empty());
    assert_eq!(
        logs.matching("context_management polyfill: unknown edit type"),
        vec![
            String::from(
                "DEBUG context_management polyfill: unknown edit type 'future' — skipping"
            ),
            String::from("DEBUG context_management polyfill: unknown edit type '7' — skipping"),
            String::from("DEBUG context_management polyfill: unknown edit type 'None' — skipping"),
        ]
    );
}

#[tokio::test]
async fn unknown_edit_type_values_print_like_python_scalars() {
    let editor = ScriptedEditor::new(Vec::new(), Vec::new());
    let spec = parse(
        r#"{"edits":[{"type":null},{"type":true},{"type":false},
            {"type":[1,"x"]},{"type":{"a":1}}]}"#,
    );
    let (logs, guard) = capture_logs();
    let dispatch = request(decided(M0), None, Some(&spec));
    let result = apply_context_management(dispatch, &editor)
        .await
        .expect("an unknown edit type only logs");
    drop(guard);

    assert_eq!(result.messages, decided(M0));
    assert!(editor.calls().is_empty());
    let messages = logs.matching("context_management polyfill: unknown edit type");
    assert_eq!(
        messages,
        vec![
            String::from("DEBUG context_management polyfill: unknown edit type 'None' — skipping"),
            String::from("DEBUG context_management polyfill: unknown edit type 'True' — skipping"),
            String::from("DEBUG context_management polyfill: unknown edit type 'False' — skipping"),
            String::from(
                "DEBUG context_management polyfill: unknown edit type '[1,\"x\"]' — skipping"
            ),
            String::from(
                "DEBUG context_management polyfill: unknown edit type '{\"a\":1}' — skipping"
            ),
        ]
    );
}

#[tokio::test]
async fn no_edits_reports_no_diagnostics() {
    let editor = ScriptedEditor::new(Vec::new(), Vec::new());
    let messages = decided(r#"[{"role":"user","content":"hello"}]"#);
    let system = parse(BASE_SYSTEM);
    let spec = parse(r#"{"edits":[]}"#);
    let (logs, guard) = capture_logs();
    let dispatch = request(messages.clone(), Some(system.clone()), Some(&spec));
    let result = apply_context_management(dispatch, &editor)
        .await
        .expect("an empty edit list is a valid spec");
    drop(guard);

    assert_eq!(result.messages, messages);
    assert_eq!(result.system, Some(system));
    assert!(result.applied_edits.is_empty());
    assert!(result.compaction_block.is_none());
    assert!(result.iterations_usage.is_none());
    assert!(logs.lines().is_empty());
}

#[tokio::test]
async fn serial_editor_result_flow() {
    let editor = flow_editor();
    let spec = flow_spec();
    let summary_metadata = metadata();
    let router = NeverCalled;
    let request_principal = principal(r#"{"user_id":"u1"}"#);
    let dispatch = flow_dispatch(&spec, &summary_metadata, &router, &request_principal);
    let result = apply_context_management(dispatch, &editor)
        .await
        .expect("the three edits run in order");

    assert_flow_result(&result);
    let calls = editor.calls();
    assert_eq!(calls.len(), 3);
    assert_clear_call(&calls[0], M0, BASE_SYSTEM);
    assert_compact_call(&calls[1]);
    assert_clear_call(&calls[2], M2, COMPACTED_SYSTEM);
}

#[tokio::test]
async fn serial_editor_result_flow_keeps_an_empty_applied_edit() {
    let messages = decided(M0);
    let returned = decided(r#"[{"role":"user","content":"cleared"}]"#);
    let system = parse(BASE_SYSTEM);
    let editor = ScriptedEditor::new(
        vec![ScriptedResult::Legacy {
            messages: returned.clone(),
            applied: Some(AppliedEdit::default()),
        }],
        Vec::new(),
    );
    let spec = parse(r#"{"edits":[{"type":"clear_tool_uses_20250919"}]}"#);
    let dispatch = request(messages, Some(system.clone()), Some(&spec));
    let result = apply_context_management(dispatch, &editor)
        .await
        .expect("the legacy tuple wraps");

    assert_eq!(result.messages, returned);
    assert_eq!(result.system, Some(system));
    assert_eq!(result.applied_edits, vec![AppliedEdit::default()]);
}

#[tokio::test]
async fn serial_editor_result_flow_keeps_a_polyfills_null_system() {
    let messages = decided(M0);
    let returned = decided(r#"[{"role":"user","content":"compacted"}]"#);
    let editor = ScriptedEditor::new(
        Vec::new(),
        vec![ScriptedResult::Polyfilled(polyfilled(
            returned.clone(),
            None,
            Vec::new(),
            Some(block("S")),
            None,
        ))],
    );
    let spec = parse(r#"{"edits":[{"type":"compact_20260112"}]}"#);
    let dispatch = request(messages, Some(parse(BASE_SYSTEM)), Some(&spec));
    let result = apply_context_management(dispatch, &editor)
        .await
        .expect("the polyfill result passes through");

    assert_eq!(result.messages, returned);
    assert_eq!(result.system, None);
}

#[tokio::test]
async fn latest_nonnull_compaction_fields() {
    let editor = ScriptedEditor::new(
        Vec::new(),
        vec![
            ScriptedResult::Polyfilled(polyfilled(
                decided(M0),
                Some(parse(r#""FIRST""#)),
                vec![applied(1)],
                Some(block("A")),
                Some(vec![iteration(10, 4)]),
            )),
            ScriptedResult::Polyfilled(polyfilled(decided(M1), None, vec![applied(2)], None, None)),
            ScriptedResult::Polyfilled(polyfilled(
                decided(M2),
                Some(parse(r#""THIRD""#)),
                vec![applied(3)],
                Some(block("B")),
                Some(Vec::new()),
            )),
        ],
    );
    let spec = three_compactions();
    let dispatch = request(decided("[]"), None, Some(&spec));
    let result = apply_context_management(dispatch, &editor)
        .await
        .expect("all three edits run");

    assert_eq!(result.messages, decided(M2));
    assert_eq!(result.system, Some(parse(r#""THIRD""#)));
    assert_eq!(
        result.applied_edits,
        vec![applied(1), applied(2), applied(3)]
    );
    assert_eq!(result.compaction_block, Some(block("B")));
    assert_eq!(result.iterations_usage, Some(Vec::new()));
}

#[tokio::test]
async fn latest_nonnull_compaction_fields_block_without_usage() {
    let editor = ScriptedEditor::new(
        Vec::new(),
        vec![
            ScriptedResult::Polyfilled(polyfilled(
                decided(M0),
                None,
                Vec::new(),
                Some(block("A")),
                Some(vec![iteration(10, 4)]),
            )),
            ScriptedResult::Polyfilled(polyfilled(
                decided(M1),
                None,
                Vec::new(),
                Some(block("B")),
                None,
            )),
        ],
    );
    let spec = parse(r#"{"edits":[{"type":"compact_20260112"},{"type":"compact_20260112"}]}"#);
    let dispatch = request(decided("[]"), None, Some(&spec));
    let result = apply_context_management(dispatch, &editor)
        .await
        .expect("both edits run");

    assert_eq!(result.messages, decided(M1));
    assert_eq!(result.compaction_block, Some(block("B")));
    assert_eq!(result.iterations_usage, Some(vec![iteration(10, 4)]));
}

#[tokio::test]
async fn latest_nonnull_compaction_fields_block_kept_when_omitted() {
    let editor = ScriptedEditor::new(
        Vec::new(),
        vec![
            ScriptedResult::Polyfilled(polyfilled(
                decided(M0),
                None,
                Vec::new(),
                Some(block("A")),
                Some(vec![iteration(10, 4)]),
            )),
            ScriptedResult::Polyfilled(polyfilled(decided(M1), None, Vec::new(), None, None)),
        ],
    );
    let spec = parse(r#"{"edits":[{"type":"compact_20260112"},{"type":"compact_20260112"}]}"#);
    let dispatch = request(decided("[]"), None, Some(&spec));
    let result = apply_context_management(dispatch, &editor)
        .await
        .expect("both edits run");

    assert_eq!(result.messages, decided(M1));
    assert_eq!(result.compaction_block, Some(block("A")));
    assert_eq!(result.iterations_usage, Some(vec![iteration(10, 4)]));
}

#[tokio::test]
async fn editor_errors_propagate_and_stop_later_edits() {
    let editor = ScriptedEditor::new(
        vec![
            ScriptedResult::Fail {
                status_code: 400,
                message: String::from("boom"),
            },
            ScriptedResult::Legacy {
                messages: decided(M0),
                applied: None,
            },
        ],
        Vec::new(),
    );
    let spec = parse(
        r#"{"edits":[{"type":"clear_tool_uses_20250919"},
            {"type":"clear_tool_uses_20250919"}]}"#,
    );
    let dispatch = request(decided(M0), None, Some(&spec));
    let error = apply_context_management(dispatch, &editor)
        .await
        .expect_err("the first editor error stops the loop");

    match error {
        DispatchError::Editor(ScriptedError::Refused {
            status_code,
            message,
        }) => {
            assert_eq!(status_code, 400);
            assert_eq!(message, "boom");
        }
        other => panic!("expected the editor's own refusal: {other:?}"),
    }
    assert_eq!(editor.calls().len(), 1);
}
