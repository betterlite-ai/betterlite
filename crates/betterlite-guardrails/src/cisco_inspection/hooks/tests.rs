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
use crate::cisco_inspection::hooks::{
    HookArgs, PostCallArgs, async_moderation_hook, async_post_call_success_hook,
    async_pre_call_hook,
};
use crate::cisco_verdict_enforcement::finalize::FinalizeOutcome;

fn obj(json: &str) -> Fields {
    let Json::Object(fields) = Json::parse(json) else {
        panic!("expected a JSON object, got: {json}");
    };
    fields
}

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

#[tokio::test]
async fn async_pre_call_hook_skips_when_the_surface_does_not_match() {
    let guardrail = guardrail(InspectionType::Chat, "http://127.0.0.1:1".to_owned());
    let mut request_data = obj(r#"{"messages":[{"role":"user","content":"hi"}]}"#);
    let args = HookArgs {
        call_type: Some("mcp_call"),
        should_run: true,
        key_user_id: None,
    };

    let outcome = async_pre_call_hook(&guardrail, &mut request_data, args).await;
    assert!(outcome.is_none());
    assert!(applied_guardrails(&request_data).is_empty());
}

#[tokio::test]
async fn async_pre_call_hook_skips_when_should_run_guardrail_is_false() {
    let guardrail = guardrail(InspectionType::Chat, "http://127.0.0.1:1".to_owned());
    let mut request_data = obj(r#"{"messages":[{"role":"user","content":"hi"}]}"#);
    let args = HookArgs {
        call_type: None,
        should_run: false,
        key_user_id: None,
    };

    let outcome = async_pre_call_hook(&guardrail, &mut request_data, args).await;
    assert!(outcome.is_none());
}

#[tokio::test]
async fn async_pre_call_hook_dispatches_chat_and_records_the_applied_guardrail() {
    let (base, mut server) = serve(StatusCode::OK, br#"{"is_safe":true}"#.to_vec()).await;
    let guardrail = guardrail(InspectionType::Chat, base);
    let mut request_data = obj(r#"{"messages":[{"role":"user","content":"hi"}]}"#);
    let args = HookArgs {
        call_type: None,
        should_run: true,
        key_user_id: None,
    };

    let outcome = async_pre_call_hook(&guardrail, &mut request_data, args).await;
    assert!(matches!(outcome, Some(FinalizeOutcome::Allow)));
    assert_eq!(applied_guardrails(&request_data), vec!["cisco-ai-defense"]);
    server.shutdown().await;
}

#[tokio::test]
async fn async_moderation_hook_uses_the_same_dispatch_as_pre_call() {
    let (base, mut server) = serve(StatusCode::OK, br#"{"is_safe":true}"#.to_vec()).await;
    let guardrail = guardrail(InspectionType::Chat, base);
    let mut request_data = obj(r#"{"messages":[{"role":"user","content":"hi"}]}"#);
    let args = HookArgs {
        call_type: None,
        should_run: true,
        key_user_id: None,
    };

    let outcome = async_moderation_hook(&guardrail, &mut request_data, args).await;
    assert!(matches!(outcome, Some(FinalizeOutcome::Allow)));
    assert_eq!(applied_guardrails(&request_data), vec!["cisco-ai-defense"]);
    server.shutdown().await;
}

#[tokio::test]
async fn async_post_call_success_hook_ignores_a_non_chat_inspection_type() {
    let guardrail = guardrail(InspectionType::Mcp, "http://127.0.0.1:1".to_owned());
    let mut request_data = obj("{}");
    let mut response = obj(r#"{"choices":[{"message":{"content":"hi"}}]}"#);
    let args = PostCallArgs {
        should_run: true,
        key_user_id: None,
    };

    let outcome =
        async_post_call_success_hook(&guardrail, &mut request_data, &mut response, args).await;
    assert!(outcome.is_none());
}

#[tokio::test]
async fn async_post_call_success_hook_dispatches_the_output_direction() {
    let (base, mut server) = serve(
        StatusCode::OK,
        br#"{"action":"sanitize","sanitized_text":"[REDACTED]"}"#.to_vec(),
    )
    .await;
    let guardrail = guardrail(InspectionType::Chat, base);
    let mut request_data = obj(r#"{"messages":[{"role":"user","content":"hi"}]}"#);
    let mut response = obj(r#"{"choices":[{"message":{"content":"there"}}]}"#);
    let args = PostCallArgs {
        should_run: true,
        key_user_id: None,
    };

    let outcome =
        async_post_call_success_hook(&guardrail, &mut request_data, &mut response, args).await;
    assert!(matches!(outcome, Some(FinalizeOutcome::Redacted(_))));
    assert_eq!(applied_guardrails(&request_data), vec!["cisco-ai-defense"]);
    let choice = &Json::parse(response.get("choices").map_or("null", |raw| raw.get())).objects()[0];
    let message = match Json::parse(choice.get("message").map_or("null", |raw| raw.get())) {
        Json::Object(fields) => fields,
        other @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            panic!("expected message to be an object, got {other:?}")
        }
    };
    assert!(matches!(
        Json::parse(message.get("content").map_or("null", |raw| raw.get())),
        Json::Text(content) if content == "[REDACTED]"
    ));
    let request_message =
        &Json::parse(request_data.get("messages").map_or("null", |raw| raw.get())).objects()[0];
    assert!(matches!(
        Json::parse(request_message.get("content").map_or("null", |raw| raw.get())),
        Json::Text(content) if content == "hi"
    ));
    server.shutdown().await;
}

#[tokio::test]
async fn async_post_call_success_hook_skips_when_the_response_has_no_scannable_content() {
    let guardrail = guardrail(InspectionType::Chat, "http://127.0.0.1:1".to_owned());
    let mut request_data = obj("{}");
    let mut response = obj(r#"{"choices":[]}"#);
    let args = PostCallArgs {
        should_run: true,
        key_user_id: None,
    };

    let outcome =
        async_post_call_success_hook(&guardrail, &mut request_data, &mut response, args).await;
    assert!(outcome.is_none());
}
