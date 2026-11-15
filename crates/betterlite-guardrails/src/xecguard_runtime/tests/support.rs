use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::net::TcpListener as StdTcpListener;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use axum::Router;
use axum::body::Bytes;
use axum::http::{StatusCode, Uri};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::xecguard_runtime::Settings as RuntimeSettings;
use betterlite_http::proxy::base_builder;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::runtime::Builder;

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::third_party_init_b::xecguard::XecGuardArgs;
use crate::xecguard_primitives::DEFAULT_POLICIES;

use super::super::XecGuardGuard;

pub(super) const GUARDRAIL_NAME: &str = "xecguard-guard";
pub(super) const SAFE: &str = r#"{"decision":"SAFE","trace_id":"trace-1"}"#;
pub(super) const UNSAFE: &str = r#"{"decision":"UNSAFE","trace_id":"trace-2","xecguard_result":[{"violated_policy_name":"HarmfulContent","rationale":"forbidden"}]}"#;
pub(super) const USER_REQUEST: &str =
    r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}]}"#;

pub(super) type Reply = (u16, &'static str);

#[derive(Clone)]
pub(super) struct Seen {
    pub(super) path: String,
    pub(super) body: GuardrailValue,
}

pub(super) struct Mock {
    pub(super) base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    _server: JoinHandle<()>,
}

pub(super) fn mock(scan: Reply, grounding: Reply) -> Mock {
    let listener = StdTcpListener::bind("127.0.0.1:0").expect("the listener");
    let base = format!("http://{}", listener.local_addr().expect("the address"));
    listener.set_nonblocking(true).expect("nonblocking");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let server = thread::Builder::new()
        .spawn(move || serve(listener, sink, scan, grounding))
        .expect("the mock server thread");
    Mock {
        base,
        seen,
        _server: server,
    }
}

fn reply_for(path: &str, scan: Reply, grounding: Reply) -> Reply {
    match path {
        "/xecguard/v1/scan" => scan,
        "/xecguard/v1/grounding" => grounding,
        _other => (404, "{}"),
    }
}

fn serve(listener: StdTcpListener, seen: Arc<Mutex<Vec<Seen>>>, scan: Reply, grounding: Reply) {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the mock runtime");
    runtime.block_on(async move {
        let tokio_listener = TcpListener::from_std(listener).expect("the tokio listener");
        let handler = move |uri: Uri, bytes: Bytes| {
            let sink = Arc::clone(&seen);
            let reply = reply_for(uri.path(), scan, grounding);
            async move {
                let body = serde_json::from_slice::<GuardrailValue>(&bytes)
                    .unwrap_or(GuardrailValue::Null);
                sink.lock().expect("the captured requests").push(Seen {
                    path: uri.path().to_owned(),
                    body,
                });
                let status = StatusCode::from_u16(reply.0).expect("the reply status");
                (status, [("content-type", "application/json")], reply.1)
            }
        };
        axum::serve(tokio_listener, Router::new().fallback(handler))
            .into_future()
            .await
            .expect("the mock server");
    });
}

pub(super) fn seen(mock: &Mock) -> Vec<Seen> {
    mock.seen.lock().expect("the captured requests").clone()
}

pub(super) fn args() -> XecGuardArgs {
    XecGuardArgs {
        name: GUARDRAIL_NAME.to_owned(),
        api_base: None,
        api_key: Some(SecretString::from("key-from-args")),
        xecguard_model: None,
        policy_names: None,
        block_on_error: None,
        grounding_strictness: None,
        timeout: None,
        event_hook: LitellmParamsMode::One("pre_call".to_owned()),
        default_on: false,
    }
}

pub(super) fn environment() -> RuntimeSettings {
    RuntimeSettings {
        api_key: Some(SecretString::from("key-from-environment")),
        api_base: Some("http://from-environment.test".to_owned()),
        block_on_error: Some("false".to_owned()),
    }
}

pub(super) fn base(name: Option<&str>, timeout: Option<f64>) -> GuardrailBase {
    GuardrailBase {
        settings: settings(name, timeout),
        mode: None,
    }
}

fn settings(name: Option<&str>, timeout: Option<f64>) -> GuardrailSettings {
    GuardrailSettings {
        guardrail_name: name.map(str::to_owned),
        supported_event_hooks: None,
        event_hook: None,
        default_on: false,
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
        timeout,
        switches: GuardrailSwitches {
            use_native_during_call_hook: false,
            use_native_lifecycle_hooks: false,
            records_own_information: false,
            uses_apply_guardrail_interface: true,
            deny_text_on_error: false,
        },
    }
}

pub(super) fn client() -> reqwest::Client {
    base_builder().build().expect("the client")
}

pub(super) fn new_guard(
    args: &XecGuardArgs,
    settings: &RuntimeSettings,
    base: GuardrailBase,
) -> XecGuardGuard {
    XecGuardGuard::new(args, settings, base, client()).expect("the guard")
}

pub(super) fn guard_at(api_base: &str) -> XecGuardGuard {
    guard_named(api_base, Some(GUARDRAIL_NAME))
}

pub(super) fn guard_named(api_base: &str, name: Option<&str>) -> XecGuardGuard {
    let mut input = args();
    input.api_base = Some(api_base.to_owned());
    new_guard(&input, &RuntimeSettings::default(), base(name, None))
}

pub(super) fn guard_with_block(api_base: &str, block_on_error: bool) -> XecGuardGuard {
    let mut input = args();
    input.api_base = Some(api_base.to_owned());
    input.block_on_error = Some(block_on_error);
    new_guard(
        &input,
        &RuntimeSettings::default(),
        base(Some(GUARDRAIL_NAME), None),
    )
}

pub(super) fn default_policies() -> Vec<String> {
    DEFAULT_POLICIES
        .iter()
        .map(|policy| (*policy).to_owned())
        .collect()
}

pub(super) fn inputs(texts: &[&str]) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: Some(texts.iter().map(|text| (*text).to_owned()).collect()),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    }
}

pub(super) fn request_data(json: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(json).expect("the request data")
}

pub(super) fn messages_of(body: &GuardrailValue) -> Vec<GuardrailValue> {
    if let GuardrailValue::Array(messages) = field(body, "messages") {
        return messages.clone();
    }
    Vec::new()
}

pub(super) fn field<'value>(value: &'value GuardrailValue, name: &str) -> &'value GuardrailValue {
    if let GuardrailValue::Object(fields) = value {
        return fields.get(name).unwrap_or(&GuardrailValue::Null);
    }
    &GuardrailValue::Null
}

pub(super) fn text(value: &GuardrailValue) -> &str {
    if let GuardrailValue::Text(text) = value {
        return text;
    }
    ""
}

pub(super) fn texts(value: &GuardrailValue) -> Vec<String> {
    if let GuardrailValue::Array(items) = value {
        return items.iter().filter_map(text_of).collect();
    }
    Vec::new()
}

fn text_of(value: &GuardrailValue) -> Option<String> {
    if let GuardrailValue::Text(text) = value {
        return Some(text.clone());
    }
    None
}
