use std::collections::BTreeMap;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_prompts::arguments::json::Json as PromptJson;
use betterlite_prompts::file_ids::mapping::FileIdMapping;

use super::builders::{
    call, failure, field, header_values, json, kwargs, list, object, response_json, run_create,
    text, tools,
};
use super::support::{
    Gateway, Harness, NoInterception, Nothing, OK_RESPONSE, PromptMark, RecordingBridge, Rig, Seams,
};
use crate::api::response_id_traversal::response_id::transcode;
use crate::api::responses_create::error::CreateError;
use crate::api::responses_provider_selection::provider_config::CompatibilityFailure;

pub(crate) async fn affinity_headers() {
    let rig = Rig::new();
    rig.wire.queue(200, OK_RESPONSE);
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
    let mut from_metadata = call("openai/gpt-4o");
    from_metadata.kwargs = kwargs(r#"{"provider_affinity_header":"X-Session"}"#);
    from_metadata.metadata = Some(json(r#"{"session_id":"meta-s"}"#));
    run_create(&seams.deps(), &from_metadata, &mut harness)
        .await
        .expect("the metadata session answers");
    assert_affinity_headers(&rig);
}

fn assert_affinity_headers(rig: &Rig) {
    let requests = rig.wire.requests();
    assert_eq!(
        header_values(&requests[0], "x-session"),
        vec![String::from("s")]
    );
    assert_eq!(
        header_values(&requests[1], "x-session"),
        vec![String::from("existing")]
    );
    assert_eq!(
        header_values(&requests[2], "x-session"),
        vec![String::from("meta-s")]
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
        .err()
        .expect("the control characters fail the call");
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
    assert_mcp_dispatch(&gateway, &rig);
    let body = object(&response_json(&created));
    assert_eq!(
        text(&body, "id"),
        "resp_67ccd2bed1ec8190b14f964abc0542670bb6a6b452d3795b"
    );
    skip_mcp_truthiness(&nothing, &mut harness).await;
}

fn assert_mcp_dispatch(gateway: &Gateway, rig: &Rig) {
    assert_eq!(
        rig.events.recorded(),
        vec![String::from("prompt"), String::from("mcp")]
    );
    assert!(rig.wire.requests().is_empty());
    let record = gateway.mcp_record().expect("the MCP dispatch ran");
    assert_eq!(record.input, json(r#""hello""#));
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
    let code_tool: PromptJson = transcode(&reached[1]).expect("the tool converts");
    let container = object(field(&object(&code_tool), "container"));
    assert_eq!(
        list(field(&container, "file_ids")),
        vec![PromptJson::Text(String::from("provider-file-xyz"))]
    );
}
async fn skip_mcp_truthiness(nothing: &Nothing, harness: &mut Harness) {
    let skipping = Rig::new();
    skipping.wire.queue(200, OK_RESPONSE);
    let skipping_gateway = Gateway::requesting(&skipping.events);
    let skipping_bridge = RecordingBridge::new(&skipping.events);
    let skipping_seams = Seams::new(&skipping, nothing, &skipping_gateway, &skipping_bridge);
    let mut skipped = call("openai/gpt-4o");
    skipped.kwargs = kwargs(r#"{"_skip_mcp_handler":true}"#);
    run_create(&skipping_seams.deps(), &skipped, harness)
        .await
        .expect("a skipped MCP call reaches the provider");
    assert!(skipping_gateway.mcp_record().is_none());
    assert_eq!(skipping.wire.requests().len(), 1);

    for falsy in [
        r#"{"_skip_mcp_handler":0}"#,
        r#"{"_skip_mcp_handler":""}"#,
        r#"{"_skip_mcp_handler":[]}"#,
        r#"{"_skip_mcp_handler":{}}"#,
    ] {
        let dispatching = Rig::new();
        let dispatching_gateway = Gateway::requesting(&dispatching.events);
        let dispatching_bridge = RecordingBridge::new(&dispatching.events);
        let dispatching_seams = Seams::new(
            &dispatching,
            nothing,
            &dispatching_gateway,
            &dispatching_bridge,
        );
        let mut falsy_call = call("openai/gpt-4o");
        falsy_call.kwargs = kwargs(falsy);
        run_create(&dispatching_seams.deps(), &falsy_call, harness)
            .await
            .expect("a falsy skip flag dispatches MCP");
        assert!(dispatching_gateway.mcp_record().is_some(), "{falsy}");
        assert!(dispatching.wire.requests().is_empty());
    }
}

pub(crate) async fn file_search_extras() {
    let mut rig = Rig::new();
    rig.configs.native_file_search = false;
    let gateway = Gateway::answering_file_search(&rig.events);
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
    assert_file_search_dispatch(&gateway, &rig);
    let body = object(&response_json(&created));
    assert_eq!(
        text(&body, "id"),
        "resp_67ccd2bed1ec8190b14f964abc0542670bb6a6b452d3795b"
    );
}

fn assert_file_search_dispatch(gateway: &Gateway, rig: &Rig) {
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
    assert_eq!(record.input, json(r#""hello""#));
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
}

pub(crate) async fn encrypted_support() {
    let mut rig = Rig::new();
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
        .err()
        .expect("an incapable target rejects the demand");
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

    bridged_encrypted(&nothing, &gateway, &bridge, &mut harness).await;
    let tolerant = r#"{"_require_encrypted_task_support":1}"#;
    encrypted_case(&nothing, &gateway, &mut harness, tolerant, false).await;
    let demanded = r#"{"_require_encrypted_task_support":true}"#;
    encrypted_case(&nothing, &gateway, &mut harness, demanded, true).await;
}

async fn bridged_encrypted(
    nothing: &Nothing,
    gateway: &Gateway,
    bridge: &RecordingBridge,
    harness: &mut Harness,
) {
    let bridged = Rig::new();
    let bridged_seams = Seams::new(&bridged, nothing, gateway, bridge);
    let mut chat = call("openai/gpt-4o");
    chat.kwargs =
        kwargs(r#"{"_require_encrypted_task_support":true,"use_chat_completions_api":true}"#);
    let bridged_error = run_create(&bridged_seams.deps(), &chat, harness)
        .await
        .err()
        .expect("a bridged target rejects the demand");
    assert!(matches!(
        bridged_error,
        CreateError::Compatibility(CompatibilityFailure::EncryptedTaskUnsupported)
    ));
    assert!(bridged.wire.requests().is_empty());
}

async fn encrypted_case(
    nothing: &Nothing,
    gateway: &Gateway,
    harness: &mut Harness,
    demand: &str,
    lookup_encrypted: bool,
) {
    let mut capable = Rig::new();
    capable.lookup.encrypted = lookup_encrypted;
    capable.wire.queue(200, OK_RESPONSE);
    let capable_bridge = RecordingBridge::new(&capable.events);
    let capable_seams = Seams::new(&capable, nothing, gateway, &capable_bridge);
    let mut request = call("openai/gpt-4o");
    request.kwargs = kwargs(demand);
    run_create(&capable_seams.deps(), &request, harness)
        .await
        .expect("the demand is satisfied or not made");
    assert_eq!(capable.wire.requests().len(), 1);
}
