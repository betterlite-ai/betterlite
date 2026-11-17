use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::net::TcpListener as StdTcpListener;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::response::IntoResponse;
use betterlite_config::sections::guardrail_identity::enums::PiiAction;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::presidio_runtime::Settings as RuntimeSettings;
use betterlite_http::proxy::base_builder;
use serde_json::value::RawValue;
use tokio::net::TcpListener;
use tokio::runtime::Builder;

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::guardrail_initializers::presidio::{PresidioArgs, PresidioRole};
use crate::presidio_response::wire::{Content, ResponseBody};
use crate::presidio_runtime::error::PresidioRuntimeError;
use crate::presidio_runtime::{PresidioDefaults, PresidioGuard, PresidioGuardParams};

pub(super) const GUARDRAIL_NAME: &str = "presidio-pii";
pub(super) const NOTHING_FOUND: &str = "[]";
pub(super) const MASKED: &str =
    r#"{"text":"<PHONE_NUMBER>","items":[{"entity_type":"PHONE_NUMBER"}]}"#;
pub(super) const PHONE_FOUND: &str =
    r#"[{"entity_type":"PHONE_NUMBER","start":11,"end":19,"score":0.9}]"#;
pub(super) const SOCIAL_SECURITY_FOUND: &str =
    r#"[{"entity_type":"US_SSN","start":0,"end":4,"score":0.9}]"#;

pub(super) type Reply = (u16, &'static str);

#[derive(Clone)]
pub(super) struct Seen {
    pub(super) path: String,
    pub(super) body: GuardrailValue,
}

#[derive(Clone)]
pub(super) struct Replies {
    pub(super) analyze: Reply,
    pub(super) anonymize: Reply,
}

pub(super) struct Mock {
    pub(super) base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    _server: JoinHandle<()>,
}

#[derive(Clone)]
struct MockState {
    seen: Arc<Mutex<Vec<Seen>>>,
    replies: Replies,
}

pub(super) fn mock(replies: Replies) -> Mock {
    let listener = StdTcpListener::bind("127.0.0.1:0").expect("the listener");
    let base = format!("http://{}", listener.local_addr().expect("the address"));
    listener.set_nonblocking(true).expect("nonblocking");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let server = thread::Builder::new()
        .spawn(move || serve(listener, sink, replies))
        .expect("the mock server thread");
    Mock {
        base,
        seen,
        _server: server,
    }
}

pub(super) fn masking() -> Mock {
    mock(Replies {
        analyze: (200, PHONE_FOUND),
        anonymize: (200, MASKED),
    })
}

pub(super) fn quiet() -> Mock {
    mock(Replies {
        analyze: (200, NOTHING_FOUND),
        anonymize: (200, MASKED),
    })
}

fn reply_for(path: &str, replies: &Replies) -> Reply {
    match path {
        "/analyze" => replies.analyze,
        "/anonymize" => replies.anonymize,
        _other => (404, "{}"),
    }
}

async fn answer(State(state): State<MockState>, uri: Uri, bytes: Bytes) -> impl IntoResponse {
    let body = serde_json::from_slice::<GuardrailValue>(&bytes).unwrap_or(GuardrailValue::Null);
    state
        .seen
        .lock()
        .expect("the captured requests")
        .push(Seen {
            path: uri.path().to_owned(),
            body,
        });
    let reply = reply_for(uri.path(), &state.replies);
    let status = StatusCode::from_u16(reply.0).expect("the reply status");
    (status, [("content-type", "application/json")], reply.1)
}

fn serve(listener: StdTcpListener, seen: Arc<Mutex<Vec<Seen>>>, replies: Replies) {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the mock runtime");
    runtime.block_on(async move {
        let tokio_listener = TcpListener::from_std(listener).expect("the tokio listener");
        let state = MockState { seen, replies };
        axum::serve(
            tokio_listener,
            Router::new().fallback(answer).with_state(state),
        )
        .into_future()
        .await
        .expect("the mock server");
    });
}

pub(super) fn seen(mock: &Mock) -> Vec<Seen> {
    mock.seen.lock().expect("the captured requests").clone()
}

pub(super) fn analyzer_requests(mock: &Mock) -> Vec<GuardrailValue> {
    seen(mock)
        .into_iter()
        .filter(|request| request.path == "/analyze")
        .map(|request| request.body)
        .collect()
}

pub(super) fn args(analyzer_base: &str, anonymizer_base: &str) -> PresidioArgs {
    PresidioArgs {
        role: PresidioRole::Scan,
        name: GUARDRAIL_NAME.to_owned(),
        event_hook: LitellmParamsMode::One("pre_call".to_owned()),
        output_parse_pii: None,
        presidio_ad_hoc_recognizers: None,
        mock_redacted_text: None,
        default_on: true,
        pii_entities_config: None,
        presidio_score_thresholds: None,
        presidio_analyzer_api_base: Some(analyzer_base.to_owned()),
        presidio_anonymizer_api_base: Some(anonymizer_base.to_owned()),
        presidio_language: None,
        presidio_entities_deny_list: None,
        apply_to_output: false,
        mask_response_content: false,
        timeout: None,
        analyze_chunk_size_bytes: None,
    }
}

pub(super) fn guard_args(mock: &Mock) -> PresidioArgs {
    args(&mock.base, &mock.base)
}

pub(super) fn base_for(args: &PresidioArgs) -> GuardrailBase {
    let guardrail_name = (!args.name.is_empty()).then(|| args.name.clone());
    let event_hook = Some(args.event_hook.clone());
    GuardrailBase {
        settings: GuardrailSettings {
            guardrail_name,
            supported_event_hooks: None,
            event_hook,
            default_on: args.default_on,
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
            timeout: args.timeout,
            switches: GuardrailSwitches {
                use_native_during_call_hook: false,
                use_native_lifecycle_hooks: false,
                records_own_information: false,
                uses_apply_guardrail_interface: true,
                deny_text_on_error: false,
            },
        },
        mode: None,
    }
}

pub(super) fn build(
    args: &PresidioArgs,
    defaults: PresidioDefaults,
    settings: RuntimeSettings,
    logging_only: bool,
    callback_role: Option<PresidioRole>,
) -> Result<PresidioGuard, PresidioRuntimeError> {
    let params = PresidioGuardParams {
        defaults,
        settings: &settings,
        logging_only,
        callback_role,
    };
    let base = base_for(args);
    PresidioGuard::new(args, &params, base, client())
}

fn client() -> reqwest::Client {
    base_builder().build().expect("the client")
}

fn built(
    args: &PresidioArgs,
    settings: RuntimeSettings,
    logging_only: bool,
    callback_role: Option<PresidioRole>,
) -> PresidioGuard {
    build(
        args,
        PresidioDefaults::default(),
        settings,
        logging_only,
        callback_role,
    )
    .expect("the guard")
}

pub(super) fn defaults(analyzer_base: &str, anonymizer_base: &str) -> PresidioDefaults {
    PresidioDefaults {
        analyzer_api_base: Some(analyzer_base.to_owned()),
        anonymizer_api_base: Some(anonymizer_base.to_owned()),
    }
}

pub(super) fn guard(args: &PresidioArgs) -> PresidioGuard {
    built(args, RuntimeSettings::default(), false, Some(args.role))
}

pub(super) fn guard_with(args: &PresidioArgs, settings: RuntimeSettings) -> PresidioGuard {
    built(args, settings, false, Some(args.role))
}

pub(super) fn roleless(args: &PresidioArgs) -> PresidioGuard {
    built(args, RuntimeSettings::default(), false, None)
}

pub(super) fn logging_only_guard(args: &PresidioArgs) -> PresidioGuard {
    built(args, RuntimeSettings::default(), true, None)
}

pub(super) fn restore_guard(mock: &Mock) -> PresidioGuard {
    let mut args = guard_args(mock);
    args.role = PresidioRole::Restore;
    guard(&args)
}

pub(super) fn output_masking_roleless_guard(mock: &Mock) -> PresidioGuard {
    let mut args = guard_args(mock);
    args.apply_to_output = true;
    roleless(&args)
}

pub(super) fn guard_at(mock: &Mock) -> PresidioGuard {
    guard(&guard_args(mock))
}

pub(super) fn request(json: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(json).expect("the request data")
}

pub(super) fn inputs(texts: Option<Vec<&str>>) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: texts.map(|listed| listed.into_iter().map(str::to_owned).collect()),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    }
}

pub(super) fn tokens(stored: &[(&str, &str)]) -> BTreeMap<String, GuardrailValue> {
    let pii_tokens: BTreeMap<String, GuardrailValue> = stored
        .iter()
        .map(|(token, original)| {
            (
                (*token).to_owned(),
                GuardrailValue::Text((*original).to_owned()),
            )
        })
        .collect();
    BTreeMap::from([(
        "metadata".to_owned(),
        GuardrailValue::Object(BTreeMap::from([(
            "pii_tokens".to_owned(),
            GuardrailValue::Object(pii_tokens),
        )])),
    )])
}

pub(super) fn messages_of(data: &BTreeMap<String, GuardrailValue>) -> Vec<GuardrailValue> {
    let Some(GuardrailValue::Array(messages)) = data.get("messages") else {
        return Vec::new();
    };
    messages.clone()
}

pub(super) fn content_of(message: &GuardrailValue) -> GuardrailValue {
    field(message, "content").clone()
}

pub(super) fn content_parts(message: &GuardrailValue) -> Vec<GuardrailValue> {
    let GuardrailValue::Array(parts) = content_of(message) else {
        return Vec::new();
    };
    parts
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

pub(super) fn pii_tokens_of(data: &BTreeMap<String, GuardrailValue>) -> BTreeMap<String, String> {
    let metadata = data
        .get("metadata")
        .cloned()
        .unwrap_or(GuardrailValue::Null);
    let tokens = field(&metadata, "pii_tokens");
    if let GuardrailValue::Object(entries) = tokens {
        return entries
            .iter()
            .map(|(token, original)| (token.clone(), text(original).to_owned()))
            .collect();
    }
    BTreeMap::new()
}

pub(super) fn block(entity_type: &str) -> BTreeMap<String, PiiAction> {
    BTreeMap::from([(entity_type.to_owned(), PiiAction::Block)])
}

pub(super) fn example_recognizers_path() -> String {
    format!(
        "{}/src/presidio_runtime/tests/example_presidio_ad_hoc_recognizer.json",
        env!("CARGO_MANIFEST_DIR")
    )
}

pub(super) fn response_body(json: &str) -> ResponseBody {
    ResponseBody::from_raw(RawValue::from_string(json.to_owned()).expect("the raw response"))
        .expect("the response body")
}

pub(super) fn response_content(response: &ResponseBody) -> Option<String> {
    let ResponseBody::Chat(chat) = response else {
        return None;
    };
    let choice = chat.choices.as_ref()?.first()?;
    let message = choice.message.as_ref()?;
    match message.content.as_ref()? {
        Content::Text(text) => Some(text.clone()),
        Content::Blocks(_) | Content::Other(_) => None,
    }
}

pub(super) fn mask_with_mock(args: &mut PresidioArgs) {
    args.mock_redacted_text = Some(BTreeMap::from([(
        "text".to_owned(),
        GuardrailValue::Text("MASKED".to_owned()),
    )]));
}
