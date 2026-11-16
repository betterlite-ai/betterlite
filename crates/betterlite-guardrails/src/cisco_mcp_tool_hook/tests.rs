#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io;
use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use axum::routing::post;
use betterlite_config::sections::cisco_verdict_contracts::{
    Configured, FallbackAction, FlaggedAction, InspectionType,
};
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::mcp_tool_hook::types::{
    McpHiddenParams, McpPostCallResponse, McpToolHookCall, McpToolResult,
};
use betterlite_models::decisions::Json as DecisionsJson;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;
use crate::cisco_inspection::construct::CiscoInspectionGuardrail;
use crate::cisco_mcp::context::hydrate_tool_context;
use crate::cisco_mcp_tool_hook::CiscoMcpToolHook;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};

fn guardrail(inspection_type: InspectionType, api_base: String) -> CiscoInspectionGuardrail {
    let settings = Configured {
        guardrail_name: "cisco-ai-defense".to_owned(),
        api_key: SecretString::from("key".to_owned()),
        api_base,
        inspection_type,
        inspect_path: "/inspect".to_owned(),
        enabled_rules: Vec::new(),
        on_flagged_action: FlaggedAction::Block,
        fallback_on_error: FallbackAction::Block,
        timeout: 10.0,
        integration_profile_id: None,
        integration_profile_version: None,
        integration_tenant_id: None,
        integration_type: None,
    };
    CiscoInspectionGuardrail::new(settings).expect("default TLS backend builds a client")
}

fn hook(guardrail: CiscoInspectionGuardrail, mode: &str) -> CiscoMcpToolHook {
    let event_hook: LitellmParamsMode = serde_json::from_str(mode).expect("gate mode");
    CiscoMcpToolHook {
        guardrail: Arc::new(guardrail),
        selection: GuardrailBase {
            settings: GuardrailSettings {
                guardrail_name: Some("cisco-ai-defense".to_owned()),
                event_hook: Some(event_hook),
                default_on: true,
                supported_event_hooks: None,
                mask_request_content: false,
                mask_response_content: false,
                violation_message_template: None,
                end_session_after_n_fails: None,
                on_violation: None,
                realtime_violation_message: None,
                on_sensitive_data: None,
                sensitive_data_route_to_model: None,
                sticky_session_routing: true,
                run_in_parallel: false,
                scan_raw_request: false,
                only_scan_new_messages: false,
                timeout: None,
                switches: GuardrailSwitches::default(),
            },
            mode: None,
        },
    }
}

async fn serve(status: StatusCode, body: Vec<u8>) -> (String, JoinSet<io::Result<()>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let router = Router::new().route(
        "/inspect",
        post(move || {
            let reply = body.clone();
            async move { (status, reply) }
        }),
    );
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, server)
}

fn kwargs(json: &str) -> IndexMap<String, DecisionsJson> {
    serde_json::from_str(json).expect("kwargs parse")
}

fn prepared(kwargs: &IndexMap<String, DecisionsJson>) -> Fields {
    let mut data = super::request::request_fields(kwargs);
    hydrate_tool_context(&mut data);
    data
}

fn text_item(text: &str) -> DecisionsJson {
    let mut fields = IndexMap::new();
    fields.insert("type".to_owned(), DecisionsJson::Text("text".to_owned()));
    fields.insert("text".to_owned(), DecisionsJson::Text(text.to_owned()));
    DecisionsJson::Object(fields)
}

fn tool_result() -> McpToolResult {
    McpToolResult {
        content: vec![text_item("secret")],
        structured_content: None,
        is_error: false,
    }
}

fn time() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2024-01-01T00:00:00+00:00").expect("timestamp")
}

fn call<'call>(
    kwargs: &'call IndexMap<String, DecisionsJson>,
    response: &'call mut McpPostCallResponse,
    original: &'call mut McpToolResult,
) -> McpToolHookCall<'call> {
    McpToolHookCall {
        kwargs,
        response,
        original,
        start_time: time(),
        end_time: time(),
    }
}

fn applied_guardrails(request_data: &Fields) -> Vec<String> {
    let bucket_key = if request_data.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    };
    match Json::parse(request_data.get(bucket_key).map_or("null", |raw| raw.get())) {
        Json::Object(bucket) => match Json::parse(
            bucket
                .get("applied_guardrails")
                .map_or("null", |raw| raw.get()),
        ) {
            Json::List(items) => items
                .iter()
                .filter_map(|item| match Json::parse(item.get()) {
                    Json::Text(name) => Some(name),
                    Json::Null
                    | Json::Bool(_)
                    | Json::Number(_)
                    | Json::List(_)
                    | Json::Object(_) => None,
                })
                .collect(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
                Vec::new()
            }
        },
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => Vec::new(),
    }
}

fn response_copy() -> McpPostCallResponse {
    McpPostCallResponse {
        mcp_tool_call_response: vec![text_item("secret")],
        hidden_params: McpHiddenParams {
            response_cost: Some(0.25),
        },
    }
}

fn blocking_text(reply: &McpPostCallResponse) -> String {
    let DecisionsJson::Object(fields) = &reply.mcp_tool_call_response[0] else {
        panic!("expected an object item");
    };
    assert_eq!(
        fields.get("type"),
        Some(&DecisionsJson::Text("text".to_owned()))
    );
    let DecisionsJson::Text(text) = fields.get("text").expect("text") else {
        panic!("expected a text item");
    };
    text.clone()
}

#[test]
fn copies_the_listed_kwargs_and_skips_null_ones() {
    let kwargs = kwargs(
        r#"{"unlisted":"dropped","guardrails":["cisco-ai-defense"],"mcp_tool_call_metadata":{"name":"x"},"litellm_metadata":{"l":"2"},"metadata":{"m":"1"},"server_name":"srv2","mcp_server_name":"srv","arguments":{"b":"2"},"mcp_arguments":{"a":"1"},"tool_name":"tool2","mcp_tool_name":"tool","user":null,"id":"id-1","litellm_call_id":"call-1","name":"n"}"#,
    );

    let data = super::request::request_fields(&kwargs);

    assert_eq!(
        serde_json::to_string(&data).expect("fields serialize"),
        r#"{"name":"n","litellm_call_id":"call-1","id":"id-1","mcp_tool_name":"tool","tool_name":"tool2","mcp_arguments":{"a":"1"},"arguments":{"b":"2"},"mcp_server_name":"srv","server_name":"srv2","metadata":{"m":"1"},"litellm_metadata":{"l":"2"},"mcp_tool_call_metadata":{"name":"x"},"guardrails":["cisco-ai-defense"]}"#
    );
}

#[test]
fn hydrates_the_tool_context_from_the_call_metadata() {
    let kwargs = kwargs(
        r#"{"mcp_tool_name":"get_weather","mcp_tool_call_metadata":{"name":"get_weather","mcp_server_name":"srv"}}"#,
    );

    let data = prepared(&kwargs);

    assert_eq!(
        serde_json::to_string(&data).expect("fields serialize"),
        r#"{"mcp_tool_name":"get_weather","mcp_tool_call_metadata":{"name":"get_weather","mcp_server_name":"srv"},"tool_name":"get_weather","name":"get_weather","mcp_server_name":"srv","server_name":"srv"}"#
    );
}

#[test]
fn runs_when_either_mcp_gate_event_is_on() {
    let during = hook(
        guardrail(InspectionType::Mcp, "http://127.0.0.1:1".to_owned()),
        "\"during_mcp_call\"",
    );
    let before = hook(
        guardrail(InspectionType::Mcp, "http://127.0.0.1:1".to_owned()),
        "\"pre_mcp_call\"",
    );

    assert!(during.runs(&BTreeMap::new()).expect("gate"));
    assert!(before.runs(&BTreeMap::new()).expect("gate"));
}

#[test]
fn skips_when_both_mcp_gate_events_are_off() {
    let hook = hook(
        guardrail(InspectionType::Mcp, "http://127.0.0.1:1".to_owned()),
        "\"post_call\"",
    );

    assert!(!hook.runs(&BTreeMap::new()).expect("gate"));
}

#[tokio::test]
async fn a_scan_that_the_gate_skips_records_nothing() {
    let hook = hook(
        guardrail(InspectionType::Mcp, "http://127.0.0.1:1".to_owned()),
        "\"post_call\"",
    );
    let kwargs = kwargs(r#"{"mcp_tool_name":"get_weather","metadata":{}}"#);
    let mut request_data = prepared(&kwargs);
    let mut response = response_copy();
    let mut original = tool_result();

    let result = hook
        .inspect_tool_output(
            &mut request_data,
            call(&kwargs, &mut response, &mut original),
        )
        .await
        .expect("hook");

    assert!(result.is_none());
    assert!(applied_guardrails(&request_data).is_empty());
    assert_eq!(response.mcp_tool_call_response, vec![text_item("secret")]);
}

#[tokio::test]
async fn a_chat_guardrail_answers_none() {
    let hook = hook(
        guardrail(InspectionType::Chat, "http://127.0.0.1:1".to_owned()),
        "\"during_mcp_call\"",
    );
    let kwargs = kwargs(r#"{"mcp_tool_name":"get_weather"}"#);
    let mut response = response_copy();
    let mut original = tool_result();

    let result = hook
        .async_post_mcp_tool_call_hook(call(&kwargs, &mut response, &mut original))
        .await
        .expect("hook");

    assert!(result.is_none());
    assert_eq!(response.mcp_tool_call_response, vec![text_item("secret")]);
}

#[tokio::test]
async fn an_allowed_scan_answers_none_and_records() {
    let (base, _server) = serve(StatusCode::OK, br#"{"action":"allow"}"#.to_vec()).await;
    let hook = hook(guardrail(InspectionType::Mcp, base), "\"during_mcp_call\"");
    let kwargs = kwargs(r#"{"mcp_tool_name":"get_weather","metadata":{}}"#);
    let mut request_data = prepared(&kwargs);
    let mut response = response_copy();
    let mut original = tool_result();

    let result = hook
        .inspect_tool_output(
            &mut request_data,
            call(&kwargs, &mut response, &mut original),
        )
        .await
        .expect("hook");

    assert!(result.is_none());
    assert_eq!(applied_guardrails(&request_data), vec!["cisco-ai-defense"]);
    assert_eq!(response.mcp_tool_call_response, vec![text_item("secret")]);
    assert_eq!(original.content, vec![text_item("secret")]);
}

#[tokio::test]
async fn a_block_replaces_the_copy_and_the_original() {
    let (base, _server) = serve(StatusCode::OK, br#"{"action":"block"}"#.to_vec()).await;
    let hook = hook(guardrail(InspectionType::Mcp, base), "\"during_mcp_call\"");
    let kwargs = kwargs(r#"{"mcp_tool_name":"get_weather","original_response":{},"metadata":{}}"#);
    let mut request_data = prepared(&kwargs);
    let mut response = response_copy();
    let mut original = tool_result();

    let result = hook
        .inspect_tool_output(
            &mut request_data,
            call(&kwargs, &mut response, &mut original),
        )
        .await
        .expect("hook");

    let reply = result.expect("blocking reply");
    assert_eq!(reply.hidden_params.response_cost, Some(0.25));
    assert_eq!(reply.mcp_tool_call_response.len(), 1);
    let text = blocking_text(&reply);
    assert!(text.contains("Blocked by Cisco AI Defense Guardrail"));
    assert!(text.contains("\"surface\":\"mcp\""));
    assert_eq!(
        response.mcp_tool_call_response,
        reply.mcp_tool_call_response
    );
    assert_eq!(original.content, reply.mcp_tool_call_response);
    assert_eq!(applied_guardrails(&request_data), vec!["cisco-ai-defense"]);
}

#[tokio::test]
async fn a_block_without_an_original_replaces_only_the_copy() {
    let (base, _server) = serve(StatusCode::OK, br#"{"action":"block"}"#.to_vec()).await;
    let hook = hook(guardrail(InspectionType::Mcp, base), "\"during_mcp_call\"");
    let kwargs = kwargs(r#"{"mcp_tool_name":"get_weather","metadata":{}}"#);
    let mut request_data = prepared(&kwargs);
    let mut response = response_copy();
    let mut original = tool_result();

    let result = hook
        .inspect_tool_output(
            &mut request_data,
            call(&kwargs, &mut response, &mut original),
        )
        .await
        .expect("hook");

    let reply = result.expect("blocking reply");
    assert_eq!(
        response.mcp_tool_call_response,
        reply.mcp_tool_call_response
    );
    assert_eq!(original.content, vec![text_item("secret")]);
    assert_eq!(applied_guardrails(&request_data), vec!["cisco-ai-defense"]);
}

#[tokio::test]
async fn a_missing_tool_name_skips_and_records() {
    let hook = hook(
        guardrail(InspectionType::Mcp, "http://127.0.0.1:1".to_owned()),
        "\"during_mcp_call\"",
    );
    let kwargs = kwargs(r#"{"metadata":{}}"#);
    let mut request_data = prepared(&kwargs);
    let mut response = response_copy();
    let mut original = tool_result();

    let result = hook
        .inspect_tool_output(
            &mut request_data,
            call(&kwargs, &mut response, &mut original),
        )
        .await
        .expect("hook");

    assert!(result.is_none());
    assert_eq!(applied_guardrails(&request_data), vec!["cisco-ai-defense"]);
}

#[tokio::test]
async fn a_redaction_rewrites_the_original_when_present() {
    let (base, _server) = serve(
        StatusCode::OK,
        br#"{"action":"sanitize","sanitized_text":"[REDACTED]"}"#.to_vec(),
    )
    .await;
    let hook = hook(guardrail(InspectionType::Mcp, base), "\"during_mcp_call\"");
    let kwargs = kwargs(r#"{"mcp_tool_name":"get_weather","original_response":{},"metadata":{}}"#);
    let mut request_data = prepared(&kwargs);
    let mut response = response_copy();
    let mut original = tool_result();

    let result = hook
        .inspect_tool_output(
            &mut request_data,
            call(&kwargs, &mut response, &mut original),
        )
        .await
        .expect("hook");

    assert!(result.is_none());
    assert_eq!(original.content, vec![text_item("[REDACTED]")]);
    assert_eq!(response.mcp_tool_call_response, vec![text_item("secret")]);
    assert_eq!(applied_guardrails(&request_data), vec!["cisco-ai-defense"]);
}

#[tokio::test]
async fn a_redaction_leaves_the_originals_structured_mirror_alone() {
    let (base, _server) = serve(
        StatusCode::OK,
        br#"{"action":"sanitize","sanitized_text":"[REDACTED]"}"#.to_vec(),
    )
    .await;
    let hook = hook(guardrail(InspectionType::Mcp, base), "\"during_mcp_call\"");
    let kwargs = kwargs(r#"{"mcp_tool_name":"get_weather","original_response":{},"metadata":{}}"#);
    let mut request_data = prepared(&kwargs);
    let mut response = response_copy();
    let mirror: DecisionsJson =
        serde_json::from_str(r#"{"result":"secret"}"#).expect("mirror parses");
    let mut original = McpToolResult {
        content: vec![text_item("secret")],
        structured_content: Some(mirror.clone()),
        is_error: false,
    };

    let result = hook
        .inspect_tool_output(
            &mut request_data,
            call(&kwargs, &mut response, &mut original),
        )
        .await
        .expect("hook");

    assert!(result.is_none());
    assert_eq!(original.content, vec![text_item("[REDACTED]")]);
    assert_eq!(original.structured_content, Some(mirror));
}

#[tokio::test]
async fn a_redaction_rewrites_the_copy_without_an_original() {
    let (base, _server) = serve(
        StatusCode::OK,
        br#"{"action":"sanitize","sanitized_text":"[REDACTED]"}"#.to_vec(),
    )
    .await;
    let hook = hook(guardrail(InspectionType::Mcp, base), "\"during_mcp_call\"");
    let kwargs = kwargs(r#"{"mcp_tool_name":"get_weather","metadata":{}}"#);
    let mut request_data = prepared(&kwargs);
    let mut response = response_copy();
    let mut original = tool_result();

    let result = hook
        .inspect_tool_output(
            &mut request_data,
            call(&kwargs, &mut response, &mut original),
        )
        .await
        .expect("hook");

    assert!(result.is_none());
    assert_eq!(
        response.mcp_tool_call_response,
        vec![text_item("[REDACTED]")]
    );
    assert_eq!(original.content, vec![text_item("secret")]);
    assert_eq!(applied_guardrails(&request_data), vec!["cisco-ai-defense"]);
}

#[tokio::test]
async fn a_block_leaves_the_originals_structured_mirror_and_error_flag_alone() {
    let (base, _server) = serve(StatusCode::OK, br#"{"action":"block"}"#.to_vec()).await;
    let hook = hook(guardrail(InspectionType::Mcp, base), "\"during_mcp_call\"");
    let kwargs = kwargs(r#"{"mcp_tool_name":"get_weather","original_response":{},"metadata":{}}"#);
    let mut request_data = prepared(&kwargs);
    let mut response = response_copy();
    let mirror: DecisionsJson =
        serde_json::from_str(r#"{"result":"secret"}"#).expect("mirror parses");
    let mut original = McpToolResult {
        content: vec![text_item("secret")],
        structured_content: Some(mirror.clone()),
        is_error: false,
    };

    let result = hook
        .inspect_tool_output(
            &mut request_data,
            call(&kwargs, &mut response, &mut original),
        )
        .await
        .expect("hook");

    let reply = result.expect("blocking reply");
    assert_eq!(original.content, reply.mcp_tool_call_response);
    assert_eq!(original.structured_content, Some(mirror));
    assert!(!original.is_error);
}

#[test]
fn the_callback_is_named_after_the_guardrail() {
    let hook = hook(
        guardrail(InspectionType::Mcp, "http://127.0.0.1:1".to_owned()),
        "\"during_mcp_call\"",
    );

    assert_eq!(hook.callback_name(), "cisco-ai-defense");
}

#[tokio::test]
async fn the_trait_hook_copies_the_kwargs_and_returns_the_block() {
    let (base, _server) = serve(StatusCode::OK, br#"{"action":"block"}"#.to_vec()).await;
    let hook = hook(guardrail(InspectionType::Mcp, base), "\"during_mcp_call\"");
    let kwargs = kwargs(r#"{"mcp_tool_name":"get_weather","metadata":{}}"#);
    let mut response = response_copy();
    let mut original = tool_result();

    let reply = hook
        .async_post_mcp_tool_call_hook(call(&kwargs, &mut response, &mut original))
        .await
        .expect("hook")
        .expect("blocking reply");

    assert!(blocking_text(&reply).contains("Blocked by Cisco AI Defense Guardrail"));
    assert_eq!(
        response.mcp_tool_call_response,
        reply.mcp_tool_call_response
    );
}

#[tokio::test]
async fn a_copied_guardrails_list_is_the_only_reason_an_opt_in_gate_runs() {
    let (base, _server) = serve(StatusCode::OK, br#"{"action":"block"}"#.to_vec()).await;
    let mut opt_in = hook(guardrail(InspectionType::Mcp, base), "\"during_mcp_call\"");
    opt_in.selection.settings.default_on = false;
    let requested = kwargs(
        r#"{"mcp_tool_name":"get_weather","guardrails":["cisco-ai-defense"],"metadata":{}}"#,
    );
    let unrequested = kwargs(r#"{"mcp_tool_name":"get_weather","metadata":{}}"#);
    let mut response = response_copy();
    let mut original = tool_result();

    let skipped = opt_in
        .async_post_mcp_tool_call_hook(call(&unrequested, &mut response, &mut original))
        .await
        .expect("hook");
    let scanned = opt_in
        .async_post_mcp_tool_call_hook(call(&requested, &mut response, &mut original))
        .await
        .expect("hook");

    assert!(skipped.is_none());
    assert!(scanned.is_some());
}

#[tokio::test]
async fn a_fail_closed_api_error_replaces_the_copy_and_the_original() {
    let (base, _server) = serve(StatusCode::INTERNAL_SERVER_ERROR, Vec::new()).await;
    let hook = hook(guardrail(InspectionType::Mcp, base), "\"during_mcp_call\"");
    let kwargs = kwargs(r#"{"mcp_tool_name":"get_weather","original_response":{},"metadata":{}}"#);
    let mut request_data = prepared(&kwargs);
    let mut response = response_copy();
    let mut original = tool_result();

    let result = hook
        .inspect_tool_output(
            &mut request_data,
            call(&kwargs, &mut response, &mut original),
        )
        .await
        .expect("hook");

    let reply = result.expect("blocking reply");
    let text = blocking_text(&reply);
    assert!(text.contains("CiscoAIDefenseGuardrailAPIError"));
    assert!(text.contains("fallback_on_error='block'"));
    assert_eq!(
        response.mcp_tool_call_response,
        reply.mcp_tool_call_response
    );
    assert_eq!(original.content, reply.mcp_tool_call_response);
    assert_eq!(applied_guardrails(&request_data), vec!["cisco-ai-defense"]);
}

#[tokio::test]
async fn a_fail_open_api_error_leaves_the_output_and_records() {
    let (base, _server) = serve(StatusCode::INTERNAL_SERVER_ERROR, Vec::new()).await;
    let mut open = guardrail(InspectionType::Mcp, base);
    open.settings.fallback_on_error = FallbackAction::Allow;
    let hook = hook(open, "\"during_mcp_call\"");
    let kwargs = kwargs(r#"{"mcp_tool_name":"get_weather","original_response":{},"metadata":{}}"#);
    let mut request_data = prepared(&kwargs);
    let mut response = response_copy();
    let mut original = tool_result();

    let result = hook
        .inspect_tool_output(
            &mut request_data,
            call(&kwargs, &mut response, &mut original),
        )
        .await
        .expect("hook");

    assert!(result.is_none());
    assert_eq!(response.mcp_tool_call_response, vec![text_item("secret")]);
    assert_eq!(original.content, vec![text_item("secret")]);
    assert_eq!(applied_guardrails(&request_data), vec!["cisco-ai-defense"]);
}

#[tokio::test]
async fn monitor_mode_leaves_a_flagged_output_and_records() {
    let (base, _server) = serve(StatusCode::OK, br#"{"action":"block"}"#.to_vec()).await;
    let mut monitoring = guardrail(InspectionType::Mcp, base);
    monitoring.settings.on_flagged_action = FlaggedAction::Monitor;
    let hook = hook(monitoring, "\"during_mcp_call\"");
    let kwargs = kwargs(r#"{"mcp_tool_name":"get_weather","original_response":{},"metadata":{}}"#);
    let mut request_data = prepared(&kwargs);
    let mut response = response_copy();
    let mut original = tool_result();

    let result = hook
        .inspect_tool_output(
            &mut request_data,
            call(&kwargs, &mut response, &mut original),
        )
        .await
        .expect("hook");

    assert!(result.is_none());
    assert_eq!(response.mcp_tool_call_response, vec![text_item("secret")]);
    assert_eq!(original.content, vec![text_item("secret")]);
    assert_eq!(applied_guardrails(&request_data), vec!["cisco-ai-defense"]);
}
