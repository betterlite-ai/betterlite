use std::ops::Sub as _;
use std::time::Duration;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::response_input::ResponsesInput;

use super::support::{
    BridgeRecord, Gateway, Harness, NoInterception, Nothing, OK_RESPONSE, RecordingBridge, Rig,
    Seams, body_json, call, field, json, kwargs, number_is, object, prompt_json, run_create, text,
    tools, value_number, values,
};

pub(crate) async fn bridge_selection() {
    let rig = Rig::new();
    let gateway = NoInterception;
    let bridge = RecordingBridge::new(&rig.events);
    let nothing = Nothing;
    let seams = Seams::new(&rig, &nothing, &gateway, &bridge);
    let mut prefixed = call("openai/chat_completions/gpt-4o");
    prefixed.kwargs =
        kwargs(r#"{"custom_option":"x","drop_params":true,"client_metadata":{"user":"u"}}"#);
    prefixed.allowed_openai_params = Some(vec![
        String::from("custom_option"),
        String::from("client_metadata"),
    ]);
    prefixed.extra_body = Some(object(&prompt_json(r#"{"x":1}"#)));
    let mut harness = Harness::new();
    run_create(&seams.deps(), &prefixed, &mut harness)
        .await
        .expect("the chat prefix bridges");
    assert_bridged(&rig, &bridge);

    flagged_bridges(&nothing, &gateway, &mut harness).await;
    absent_config_bridges(&nothing, &gateway, &mut harness).await;
    passing_with_and_without_tools(&nothing, &mut harness).await;
}

fn assert_bridged(rig: &Rig, bridge: &RecordingBridge) {
    assert!(rig.wire.requests().is_empty());
    assert_eq!(
        rig.events.recorded(),
        vec![String::from("config"), String::from("bridge")]
    );
    let calls = bridge.calls();
    assert_eq!(calls.len(), 1);
    let record = &calls[0];
    assert_eq!(record.model, "gpt-4o");
    assert_eq!(record.custom_llm_provider.as_deref(), Some("openai"));
    assert_bridge_flags(record);
}

fn assert_bridge_flags(record: &BridgeRecord) {
    assert!(!record.is_async);
    assert_eq!(record.stream, None);
    assert_eq!(record.input, ResponsesInput::Text(String::from("hello")));
    assert!(record.extra_headers.is_none());
    assert_bridge_kwargs(record);
}

fn assert_bridge_kwargs(record: &BridgeRecord) {
    assert_eq!(
        record.kwargs.get("custom_option"),
        Some(&JsonValue::Text(String::from("x")))
    );
    assert!(record.kwargs.contains_key("drop_params"));
    assert!(!record.kwargs.contains_key("client_metadata"));
    assert!(!record.kwargs.contains_key("use_chat_completions_api"));
    assert_eq!(
        record.kwargs.get("extra_body"),
        values(r#"[{"x":1}]"#).first()
    );
}

async fn passing_with_and_without_tools(nothing: &Nothing, harness: &mut Harness) {
    let rig = Rig::new();
    let gateway = Gateway::passing(&rig.events);
    let bridge = RecordingBridge::new(&rig.events);
    let seams = Seams::new(&rig, nothing, &gateway, &bridge);
    let mut plain = call("openai/gpt-4o");
    plain.kwargs = kwargs(r#"{"use_chat_completions_api":true}"#);
    run_create(&seams.deps(), &plain, harness)
        .await
        .expect("a bridged call without tools passes the file search by");
    assert!(gateway.file_search_record().is_none());
    assert_eq!(bridge.calls().len(), 1);
    assert!(rig.wire.requests().is_empty());

    plain.tools = Some(tools(
        r#"[{"type":"file_search","file_search":{"vector_store_ids":["vs"]}}]"#,
    ));
    run_create(&seams.deps(), &plain, harness)
        .await
        .expect("the bridged file search dispatches the emulation first");
    assert!(gateway.file_search_record().is_some());
    assert_eq!(bridge.calls().len(), 2);
    assert!(rig.wire.requests().is_empty());
}

async fn flagged_bridges(nothing: &Nothing, gateway: &NoInterception, harness: &mut Harness) {
    let flagged = Rig::new();
    let flag_bridge = RecordingBridge::new(&flagged.events);
    let flag_seams = Seams::new(&flagged, nothing, gateway, &flag_bridge);
    let mut with_flag = call("openai/gpt-4o");
    with_flag.kwargs = kwargs(r#"{"use_chat_completions_api":true}"#);
    run_create(&flag_seams.deps(), &with_flag, harness)
        .await
        .expect("the flag bridges");
    assert_eq!(flag_bridge.calls().len(), 1);
    assert!(flagged.wire.requests().is_empty());
}

async fn absent_config_bridges(nothing: &Nothing, gateway: &NoInterception, harness: &mut Harness) {
    let absent = Rig::new();
    let absent_bridge = RecordingBridge::new(&absent.events);
    let absent_seams = Seams::new(&absent, nothing, gateway, &absent_bridge);
    let other = call("anthropic/claude");
    run_create(&absent_seams.deps(), &other, harness)
        .await
        .expect("an absent native config bridges");
    let absent_calls = absent_bridge.calls();
    assert_eq!(absent_calls.len(), 1);
    assert_eq!(absent_calls[0].model, "claude");
    assert_eq!(
        absent_calls[0].custom_llm_provider.as_deref(),
        Some("anthropic")
    );
    assert!(absent.wire.requests().is_empty());
}

pub(crate) async fn reasoning() {
    let rig = Rig::new();
    let gateway = NoInterception;
    let bridge = RecordingBridge::new(&rig.events);
    let nothing = Nothing;
    let seams = Seams::new(&rig, &nothing, &gateway, &bridge);
    let mut winning = call("openai/gpt-4o");
    winning.kwargs = kwargs(r#"{"use_chat_completions_api":true,"reasoning_effort":"high"}"#);
    winning.reasoning = Some(json(r#"{"effort":"low"}"#));
    let mut harness = Harness::new();
    run_create(&seams.deps(), &winning, &mut harness)
        .await
        .expect("the bridged create answers");
    let winning_request = object(&bridge.calls()[0].request);
    let winning_mapped = object(field(&winning_request, "reasoning"));
    assert_eq!(text(&winning_mapped, "effort"), "low");
    assert!(winning_mapped.get("summary").is_none());
    assert!(winning_request.get("reasoning_effort").is_none());

    mapped_effort(&nothing, &gateway, &mut harness, true).await;
    mapped_effort(&nothing, &gateway, &mut harness, false).await;
}

async fn mapped_effort(
    nothing: &Nothing,
    gateway: &NoInterception,
    harness: &mut Harness,
    auto_summary: bool,
) {
    let rig = Rig::new();
    let bridge = RecordingBridge::new(&rig.events);
    let mut seams = Seams::new(&rig, nothing, gateway, &bridge);
    seams.reasoning_auto_summary = auto_summary;
    let mut effort = call("openai/gpt-4o");
    effort.kwargs = kwargs(r#"{"use_chat_completions_api":true,"reasoning_effort":"low"}"#);
    run_create(&seams.deps(), &effort, harness)
        .await
        .expect("the effort maps");
    let request = object(&bridge.calls()[0].request);
    let mapped = object(field(&request, "reasoning"));
    assert_eq!(text(&mapped, "effort"), "low");
    if auto_summary {
        assert_eq!(text(&mapped, "summary"), "detailed");
    } else {
        assert!(mapped.get("summary").is_none());
    }
}

pub(crate) async fn timeouts() {
    let rig = Rig::new();
    rig.wire.queue(200, OK_RESPONSE);
    rig.wire.queue(200, OK_RESPONSE);
    let gateway = NoInterception;
    let bridge = RecordingBridge::new(&rig.events);
    let nothing = Nothing;
    let seams = Seams::new(&rig, &nothing, &gateway, &bridge);
    let mut harness = Harness::new();
    run_create(&seams.deps(), &call("openai/gpt-4o"), &mut harness)
        .await
        .expect("the default timeout answers");
    let mut zero = call("openai/gpt-4o");
    zero.timeout = Some(0.0_f64);
    run_create(&seams.deps(), &zero, &mut harness)
        .await
        .expect("a zero native timeout falls back");
    let requests = rig.wire.requests();
    assert_eq!(requests[0].timeout, Duration::from_mins(100));
    assert_eq!(requests[1].timeout, Duration::from_mins(100));

    injected_and_bridged(&nothing, &gateway, &mut harness).await;
}

async fn injected_and_bridged(nothing: &Nothing, gateway: &NoInterception, harness: &mut Harness) {
    let injected = Rig::with_env(&[("REQUEST_TIMEOUT", "17")]);
    injected.wire.queue(200, OK_RESPONSE);
    let injected_bridge = RecordingBridge::new(&injected.events);
    let injected_seams = Seams::new(&injected, nothing, gateway, &injected_bridge);
    run_create(&injected_seams.deps(), &call("openai/gpt-4o"), harness)
        .await
        .expect("the injected timeout answers");
    assert_eq!(injected.wire.requests()[0].timeout, Duration::from_secs(17));

    let bridged = Rig::new();
    let bridged_bridge = RecordingBridge::new(&bridged.events);
    let bridged_seams = Seams::new(&bridged, nothing, gateway, &bridged_bridge);
    let mut bridged_zero = call("openai/gpt-4o");
    bridged_zero.kwargs = kwargs(r#"{"use_chat_completions_api":true}"#);
    bridged_zero.timeout = Some(0.0_f64);
    run_create(&bridged_seams.deps(), &bridged_zero, harness)
        .await
        .expect("a zero bridge timeout stays");
    assert!(value_number(&bridged_bridge.calls()[0].kwargs, "timeout").abs() < 1e-9_f64);

    let bridged_injected = RecordingBridge::new(&injected.events);
    let bridged_injected_seams = Seams::new(&injected, nothing, gateway, &bridged_injected);
    let mut bridged_default = call("openai/gpt-4o");
    bridged_default.kwargs = kwargs(r#"{"use_chat_completions_api":true}"#);
    run_create(&bridged_injected_seams.deps(), &bridged_default, harness)
        .await
        .expect("the bridge uses the shared default timeout");
    let timeout = value_number(&bridged_injected.calls()[0].kwargs, "timeout");
    assert!(timeout.sub(17.0_f64).abs() < 1e-9_f64);
}

pub(crate) async fn allowed_and_dropped() {
    let mut rig = Rig::new();
    rig.configs.supported = Some(vec![String::from("temperature")]);
    rig.wire.queue(200, OK_RESPONSE);
    let gateway = NoInterception;
    let bridge = RecordingBridge::new(&rig.events);
    let nothing = Nothing;
    let seams = Seams::new(&rig, &nothing, &gateway, &bridge);
    let mut first = call("openai/gpt-4o");
    first.temperature = Some(0.2_f64);
    first.service_tier = Some(String::from("flex"));
    first.kwargs = kwargs(r#"{"drop_params":true}"#);
    let mut harness = Harness::new();
    run_create(&seams.deps(), &first, &mut harness)
        .await
        .expect("the dropping create answers");
    let requests = rig.wire.requests();
    let body = body_json(&requests[0]);
    let fields = object(&body);
    number_is(&fields, "temperature", 0.2_f64);
    assert!(fields.get("service_tier").is_none());
    assert_eq!(rig.configs.dropped(), vec![true]);
}
