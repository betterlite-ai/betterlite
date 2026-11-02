#![cfg(test)]

use std::future::IntoFuture as _;
use std::io;

use axum::Router;
use axum::http::StatusCode;
use axum::routing::post;
use betterlite_config::sections::cisco_verdict_contracts::{
    Configured, FallbackAction, FlaggedAction, InspectionType,
};
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;
use crate::cisco_inspection::construct::CiscoInspectionGuardrail;
use crate::cisco_inspection::mcp::{inspect_mcp_request, inspect_mcp_response};
use crate::cisco_verdict_enforcement::finalize::FinalizeOutcome;

fn obj(json: &str) -> Fields {
    let Json::Object(fields) = Json::parse(json) else {
        panic!("expected a JSON object, got: {json}");
    };
    fields
}

fn guardrail(api_base: String) -> CiscoInspectionGuardrail {
    let settings = Configured {
        guardrail_name: "cisco-ai-defense".to_owned(),
        api_key: SecretString::from("key".to_owned()),
        api_base,
        inspection_type: InspectionType::Mcp,
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

async fn dead_base() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    drop(listener);
    base
}

#[tokio::test]
async fn inspect_mcp_request_sends_the_built_envelope_and_allows() {
    let (base, mut server) = serve(StatusCode::OK, br#"{"is_safe":true}"#.to_vec()).await;
    let guardrail = guardrail(base);
    let mut request_data = obj(r#"{"mcp_tool_name":"search"}"#);

    let outcome = inspect_mcp_request(&guardrail, &mut request_data).await;
    assert!(matches!(outcome, Some(FinalizeOutcome::Allow)));
    server.shutdown().await;
}

#[tokio::test]
async fn inspect_mcp_request_skips_without_a_call_when_no_payload_can_be_built() {
    let guardrail = guardrail(dead_base().await);
    let mut request_data = obj("{}");

    let outcome = inspect_mcp_request(&guardrail, &mut request_data).await;
    assert!(outcome.is_none());
}

#[tokio::test]
async fn inspect_mcp_response_sends_the_built_envelope_and_allows() {
    let (base, mut server) = serve(StatusCode::OK, br#"{"is_safe":true}"#.to_vec()).await;
    let guardrail = guardrail(base);
    let mut request_data = obj(r#"{"mcp_tool_name":"search"}"#);
    let response = Json::parse(r#"{"content":[]}"#);

    let outcome = inspect_mcp_response(&guardrail, &mut request_data, &response).await;
    assert!(matches!(outcome, Some(FinalizeOutcome::Allow)));
    server.shutdown().await;
}

#[tokio::test]
async fn inspect_mcp_response_skips_without_a_call_when_no_payload_can_be_built() {
    let guardrail = guardrail(dead_base().await);
    let mut request_data = obj(r#"{"mcp_tool_name":"search"}"#);
    let response = Json::parse(r#""plain text""#);

    let outcome = inspect_mcp_response(&guardrail, &mut request_data, &response).await;
    assert!(outcome.is_none());
}
