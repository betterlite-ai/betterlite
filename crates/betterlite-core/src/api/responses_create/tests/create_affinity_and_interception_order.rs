use std::collections::BTreeMap;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_prompts::arguments::json::Json as PromptJson;
use betterlite_prompts::file_ids::mapping::FileIdMapping;

use super::support::{
    Gateway, Harness, NoInterception, Nothing, OK_RESPONSE, PromptMark, RecordingBridge, Rig,
    Seams, call, failure, field, header_values, json, kwargs, list, object, response_json,
    run_create, text, tools,
};
use crate::api::responses_create::convert::reencode;
use crate::api::responses_create::error::CreateError;
use crate::api::responses_provider_selection::provider_config::CompatibilityFailure;

pub(crate) async fn affinity_headers() {
    let rig = Rig::new();
    rig.wire.queue(200, OK_RESPONSE);
    rig.wire.queue(200, OK_RESPONSE);
    let gateway = NoInterception;
    let bridge = RecordingBridge::new(&rig.events);
    let nothing = Nothing;
    let seams = Seams::new(&rig, &nothing, &gateway, &bridge);
    let mut harness = Harness::new();
    let mut first = call("openai/gpt-4o");
    first.kwargs = kwargs(r#"{"provider_affinity_header":"X-Session","session_id":"s"}"#);
    run_create(&seams.deps(), &first, &mut harness)
        .await
        .expect("the affinity create answers");
    let mut existing = call("openai/gpt-4o");
    existing.kwargs = kwargs(r#"{"provider_affinity_header":"X-Session","session_id":"s"}"#);
    existing.extra_headers = Some(BTreeMap::from([(
        String::from("x-session"),
        String::from("existing"),
    )]));
    run_create(&seams.deps(), &existing, &mut harness)
        .await
        .expect("the existing header wins");
    let requests = rig.wire.requests();
    assert_eq!(
        header_values(&requests[0], "x-session"),
        vec![String::from("s")]
    );
    assert_eq!(
        header_values(&requests[1], "x-session"),
        vec![String::from("existing")]
    );
}

pub(crate) async fn affinity_control_characters() {
    let rig = Rig::new();
    let gateway = NoInterception;
    let bridge = RecordingBridge::new(&rig.events);
    let nothing = Nothing;
    let prompt = PromptMark::new(&rig.events);
    let mut seams = Seams::new(&rig, &nothing, &gateway, &bridge);
    seams.prompts = Some(&prompt);
    let mut failing = call("openai/gpt-4o");
    failing.kwargs = kwargs(
        r#"{"provider_affinity_header":"X-Session","session_id":"bad\nvalue","mock_response":"hi"}"#,
    );
    let mut harness = Harness::new();
    let error = run_create(&seams.deps(), &failing, &mut harness)
        .await
        .expect_err("the control characters fail the call");
    assert!(matches!(error, CreateError::Affinity(_)));
    let failure = failure(&error);
    assert_eq!(failure.class, "BadRequestError");
    assert_eq!(failure.status, 400);
    assert_eq!(failure.kind, None);
    assert_eq!(failure.param, None);
    assert_eq!(failure.code, None);
    assert!(
        failure
            .message
            .contains("session_id cannot contain HTTP header control characters")
    );
    assert_eq!(rig.events.recorded(), vec![String::from("prompt")]);
    assert!(rig.wire.requests().is_empty());
    assert_eq!(rig.resolver.seen().len(), 1);
}

pub(crate) async fn mcp_first_and_mapped() {
    let rig = Rig::new();
    let gateway = Gateway::requesting(&rig.events);
    let bridge = RecordingBridge::new(&rig.events);
    let nothing = Nothing;
    let prompt = PromptMark::new(&rig.events);
    let mapping: FileIdMapping =
        serde_json::from_str(r#"{"file-abc":{"deployment_a":"provider-file-xyz"}}"#)
            .expect("the mapping is JSON");
    let mut seams = Seams::new(&rig, &nothing, &gateway, &bridge);
    seams.prompts = Some(&prompt);
    seams.file_mapping = Some(&mapping);
    let mut first = call("openai/gpt-4o");
    first.kwargs = kwargs(
        r#"{"provider_affinity_header":"X-Session","session_id":"s","model_info":{"id":"deployment_a"},"litellm_call_id":"call-9","reasoning_effort":"low"}"#,
    );
    first.tools = Some(tools(
        r#"[{"type":"file_search","file_search":{"vector_store_ids":["vs"]}},{"type":"code_interpreter","container":{"type":"auto","file_ids":["file-abc"]}}]"#,
    ));
    first.service_tier = Some(String::from("flex"));
    first.safety_identifier = Some(String::from("user-a"));
    first.text_format = Some(json(r#"{"type":"json_object"}"#));
    first.allowed_openai_params = Some(vec![String::from("temperature")]);
    let mut harness = Harness::new();
    let created = run_create(&seams.deps(), &first, &mut harness)
        .await
        .expect("the handled MCP answers");
    assert_eq!(
        rig.events.recorded(),
        vec![String::from("prompt"), String::from("mcp")]
    );
    assert!(rig.wire.requests().is_empty());
    let record = gateway.mcp_record().expect("the MCP dispatch ran");
    assert!(record.reasoning.is_none());
    assert!(
        record
            .extra_headers
            .as_ref()
            .is_some_and(|headers| headers.get("X-Session").map(String::as_str) == Some("s"))
    );
    assert_eq!(
        record.kwargs.get("litellm_call_id"),
        Some(&JsonValue::Text(String::from("call-9")))
    );
    assert!(!record.kwargs.contains_key("service_tier"));
    assert!(!record.kwargs.contains_key("text_format"));
    let reached = record.tools.expect("the tools reach MCP");
    assert_eq!(reached.len(), 2);
    let code_tool: PromptJson = reencode(&reached[1]).expect("the tool converts");
    let container = object(field(&object(&code_tool), "container"));
    assert_eq!(
        list(field(&container, "file_ids")),
        vec![PromptJson::Text(String::from("provider-file-xyz"))]
    );
    let body = object(&response_json(&created));
    assert_eq!(
        text(&body, "id"),
        "resp_67ccd2bed1ec8190b14f964abc0542670bb6a6b452d3795b"
    );
}

pub(crate) async fn file_search_extras() {
    let rig = Rig::new();
    rig.configs.native_file_search = false;
    let gateway = Gateway::passing(&rig.events);
    let bridge = RecordingBridge::new(&rig.events);
    let nothing = Nothing;
    let prompt = PromptMark::new(&rig.events);
    let mut seams = Seams::new(&rig, &nothing, &gateway, &bridge);
    seams.prompts = Some(&prompt);
    let mut first = call("openai/gpt-4o");
    first.kwargs = kwargs(r#"{"litellm_call_id":"call-9","reasoning_effort":"low"}"#);
    first.tools = Some(tools(
        r#"[{"type":"file_search","file_search":{"vector_store_ids":["vs"]}}]"#,
    ));
    first.service_tier = Some(String::from("flex"));
    first.safety_identifier = Some(String::from("user-a"));
    first.text_format = Some(json(r#"{"type":"json_object"}"#));
    first.allowed_openai_params = Some(vec![String::from("temperature")]);
    let mut harness = Harness::new();
    let created = run_create(&seams.deps(), &first, &mut harness)
        .await
        .expect("the file-search dispatch answers");
    assert_eq!(
        rig.events.recorded(),
        vec![
            String::from("prompt"),
            String::from("config"),
            String::from("file_search")
        ]
    );
    let record = gateway
        .file_search_record()
        .expect("the file-search dispatch ran");
    assert_eq!(record.reasoning, Some(json(r#"{"effort":"low"}"#)));
    assert_eq!(record.service_tier.as_deref(), Some("flex"));
    assert_eq!(record.safety_identifier.as_deref(), Some("user-a"));
    assert_eq!(record.text_format, Some(json(r#"{"type":"json_object"}"#)));
    assert_eq!(
        record.allowed_openai_params,
        Some(vec![String::from("temperature")])
    );
    assert!(!record.use_chat_completions_api);
    assert!(!record.kwargs.contains_key("litellm_call_id"));
    let body = object(&response_json(&created));
    assert_eq!(
        text(&body, "id"),
        "resp_67ccd2bed1ec8190b14f964abc0542670bb6a6b452d3795b"
    );
}

pub(crate) async fn encrypted_support() {
    let rig = Rig::new();
    rig.lookup.encrypted = false;
    let gateway = Gateway::requesting_and_passing(&rig.events);
    let bridge = RecordingBridge::new(&rig.events);
    let nothing = Nothing;
    let seams = Seams::new(&rig, &nothing, &gateway, &bridge);
    let mut first = call("openai/gpt-4o");
    first.kwargs = kwargs(r#"{"_require_encrypted_task_support":true}"#);
    first.tools = Some(tools(
        r#"[{"type":"file_search","file_search":{"vector_store_ids":["vs"]}}]"#,
    ));
    let mut harness = Harness::new();
    let error = run_create(&seams.deps(), &first, &mut harness)
        .await
        .expect_err("an incapable target rejects the demand");
    assert!(matches!(
        error,
        CreateError::Compatibility(CompatibilityFailure::EncryptedTaskUnsupported)
    ));
    assert_eq!(
        rig.events.recorded(),
        vec![String::from("mcp"), String::from("config")]
    );
    assert!(gateway.file_search_record().is_none());
    assert!(rig.wire.requests().is_empty());

    let bridged = Rig::new();
    let bridged_seams = Seams::new(&bridged, &nothing, &gateway, &bridge);
    let mut chat = call("openai/gpt-4o");
    chat.kwargs =
        kwargs(r#"{"_require_encrypted_task_support":true,"use_chat_completions_api":true}"#);
    let bridged_error = run_create(&bridged_seams.deps(), &chat, &mut harness)
        .await
        .expect_err("a bridged target rejects the demand");
    assert!(matches!(
        bridged_error,
        CreateError::Compatibility(CompatibilityFailure::EncryptedTaskUnsupported)
    ));
    assert!(bridged.wire.requests().is_empty());

    let tolerant = Rig::new();
    tolerant.lookup.encrypted = false;
    tolerant.wire.queue(200, OK_RESPONSE);
    let tolerant_bridge = RecordingBridge::new(&tolerant.events);
    let tolerant_seams = Seams::new(&tolerant, &nothing, &gateway, &tolerant_bridge);
    let mut numeric = call("openai/gpt-4o");
    numeric.kwargs = kwargs(r#"{"_require_encrypted_task_support":1}"#);
    run_create(&tolerant_seams.deps(), &numeric, &mut harness)
        .await
        .expect("only an exact true demands encrypted support");
    assert_eq!(tolerant.wire.requests().len(), 1);

    let capable = Rig::new();
    capable.wire.queue(200, OK_RESPONSE);
    let capable_bridge = RecordingBridge::new(&capable.events);
    let capable_seams = Seams::new(&capable, &nothing, &gateway, &capable_bridge);
    let mut demanded = call("openai/gpt-4o");
    demanded.kwargs = kwargs(r#"{"_require_encrypted_task_support":true}"#);
    run_create(&capable_seams.deps(), &demanded, &mut harness)
        .await
        .expect("a capable target satisfies the demand");
    assert_eq!(capable.wire.requests().len(), 1);
}
