#![cfg(test)]

use std::future::IntoFuture as _;
use std::io;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::http::StatusCode;
use axum::routing::post;
use betterlite_config::sections::cisco_verdict_contracts::{
    Configured, FallbackAction, FlaggedAction, InspectionType,
};
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use crate::cisco_configuration::records::Direction;
use crate::cisco_conversation::json::Json;
use crate::cisco_conversation::{Fields, InspectMessage, Role};
use crate::cisco_inspection::chat::{ChatInspectionArgs, inspect_chat};
use crate::cisco_inspection::construct::CiscoInspectionGuardrail;
use crate::cisco_verdict_enforcement::api_error::ApiErrorOutcome;
use crate::cisco_verdict_enforcement::finalize::{FinalizeOutcome, ResponseSurface};

fn obj(json: &str) -> Fields {
    let Json::Object(fields) = Json::parse(json) else {
        panic!("expected a JSON object, got: {json}");
    };
    fields
}

fn plain_client() -> reqwest::Client {
    reqwest::Client::default()
}

fn guardrail(api_base: String, client: reqwest::Client) -> CiscoInspectionGuardrail {
    let settings = Configured {
        guardrail_name: "cisco-ai-defense".to_owned(),
        api_key: SecretString::from("key".to_owned()),
        api_base,
        inspection_type: InspectionType::Chat,
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
    CiscoInspectionGuardrail { settings, client }
}

fn args(key_user_id: Option<&str>) -> ChatInspectionArgs<'_> {
    ChatInspectionArgs {
        messages: vec![InspectMessage {
            role: Role::User,
            content: "hello".to_owned(),
        }],
        direction: Direction::Input,
        key_user_id,
    }
}

async fn serve_capturing(
    status: StatusCode,
    body: Vec<u8>,
) -> (String, Arc<Mutex<Option<String>>>, JoinSet<io::Result<()>>) {
    let captured = Arc::new(Mutex::new(None));
    let state = Arc::clone(&captured);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let router = Router::new().route(
        "/inspect",
        post(move |payload: Bytes| {
            *state.lock().expect("lock") =
                Some(String::from_utf8(payload.to_vec()).expect("utf-8 body"));
            let reply = body.clone();
            async move { (status, reply) }
        }),
    );
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, captured, server)
}

// text_field and object_field each pull one Json variant out of a Fields map or panic.
macro_rules! typed_field {
    ($name:ident, $variant:ident, $ty:ty, $noun:literal, $other:pat) => {
        fn $name(fields: &Fields, key: &str) -> $ty {
            match Json::field(fields, key) {
                Json::$variant(value) => value,
                other @ $other => panic!("expected {key} to be {}, got {other:?}", $noun),
            }
        }
    };
}

typed_field!(
    text_field,
    Text,
    String,
    "a string",
    (Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
);
typed_field!(
    object_field,
    Object,
    Fields,
    "an object",
    (Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
);

#[tokio::test]
async fn inspect_chat_sends_the_assembled_payload_and_allows() {
    let (base, captured, mut server) =
        serve_capturing(StatusCode::OK, br#"{"is_safe":true}"#.to_vec()).await;
    let guardrail = guardrail(base, plain_client());
    let mut request_data =
        obj(r#"{"user":"alice","litellm_call_id":"call-1","metadata":{"src_app":"gateway"}}"#);

    let outcome = inspect_chat(
        &guardrail,
        args(None),
        &mut request_data,
        ResponseSurface::None,
    )
    .await;
    assert!(matches!(outcome, FinalizeOutcome::Allow));

    let body = captured
        .lock()
        .expect("lock")
        .clone()
        .expect("captured body");
    let metadata = object_field(&obj(&body), "metadata");
    assert_eq!(text_field(&metadata, "user"), "alice");
    assert_eq!(text_field(&metadata, "client_transaction_id"), "call-1");
    assert_eq!(text_field(&metadata, "src_app"), "gateway");
    server.shutdown().await;
}

#[tokio::test]
async fn inspect_chat_falls_back_to_the_key_user_id_when_no_request_user_is_set() {
    let (base, captured, mut server) =
        serve_capturing(StatusCode::OK, br#"{"is_safe":true}"#.to_vec()).await;
    let guardrail = guardrail(base, plain_client());
    let mut request_data = obj("{}");

    inspect_chat(
        &guardrail,
        args(Some("key-user")),
        &mut request_data,
        ResponseSurface::None,
    )
    .await;

    let body = captured
        .lock()
        .expect("lock")
        .clone()
        .expect("captured body");
    let metadata = object_field(&obj(&body), "metadata");
    assert_eq!(text_field(&metadata, "user"), "key-user");
    server.shutdown().await;
}

#[tokio::test]
async fn inspect_chat_wraps_a_transport_failure_into_a_blocked_api_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    drop(listener);
    let guardrail = guardrail(base, plain_client());
    let mut request_data = obj("{}");

    let outcome = inspect_chat(
        &guardrail,
        args(None),
        &mut request_data,
        ResponseSurface::None,
    )
    .await;
    assert!(matches!(
        outcome,
        FinalizeOutcome::ApiError(ApiErrorOutcome::Blocked(_))
    ));
}
