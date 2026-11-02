#![cfg(test)]

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse as _;
use tower::ServiceExt as _;

use super::catalog::{agent_fields, raw_agent_catalog};
use super::error::AgentFieldsError;
use super::types::{AgentCreateInfo, RawAgentCreateInfo};
use super::{AGENT_FIELDS_PATH, AgentFieldsState, agent_fields_routes, get_agent_fields};

#[test]
fn agent_fields_returns_all_eight_agents_in_source_order() {
    let fields = agent_fields().expect("catalog decodes");
    let types: Vec<&str> = fields
        .iter()
        .map(|entry| entry.agent_type.as_str())
        .collect();
    assert_eq!(
        types,
        [
            "a2a",
            "langflow",
            "langgraph",
            "bedrock_agentcore",
            "azure_ai_foundry",
            "pydantic_ai_agents",
            "vertex_agent_engine",
            "watsonx_orchestrate",
        ]
    );
}

#[test]
fn bedrock_agentcore_appends_bedrock_credentials_after_its_own_field() {
    let fields = agent_fields().expect("catalog decodes");
    let agent = fields
        .iter()
        .find(|entry| entry.agent_type == "bedrock_agentcore")
        .expect("bedrock_agentcore is present");
    assert_eq!(agent.credential_fields[0].key, "agent_runtime_arn");
    let inherited = &agent.credential_fields[1..];
    assert!(!inherited.is_empty());
    assert!(
        inherited
            .iter()
            .any(|field| field.key == "aws_access_key_id")
    );
    assert!(
        inherited
            .iter()
            .all(|field| field.include_in_litellm_params == Some(true))
    );
}

#[test]
fn vertex_agent_engine_appends_vertex_ai_credentials_after_its_own_field() {
    let fields = agent_fields().expect("catalog decodes");
    let agent = fields
        .iter()
        .find(|entry| entry.agent_type == "vertex_agent_engine")
        .expect("vertex_agent_engine is present");
    assert_eq!(agent.credential_fields[0].key, "reasoning_engine_id");
    let inherited = &agent.credential_fields[1..];
    assert!(!inherited.is_empty());
    assert!(inherited.iter().any(|field| field.key == "vertex_project"));
    assert!(
        inherited
            .iter()
            .all(|field| field.include_in_litellm_params == Some(true))
    );
}

#[test]
fn azure_ai_foundry_inherits_nothing_because_azure_ai_does_not_match_azure_ai_studio() {
    let fields = agent_fields().expect("catalog decodes");
    let agent = fields
        .iter()
        .find(|entry| entry.agent_type == "azure_ai_foundry")
        .expect("azure_ai_foundry is present");
    let raw = raw_agent_catalog().expect("raw catalog decodes");
    let raw_agent = raw
        .iter()
        .find(|entry| entry.agent_type == "azure_ai_foundry")
        .expect("azure_ai_foundry is present in the raw catalog");
    assert_eq!(
        agent.credential_fields.len(),
        raw_agent.credential_fields.len()
    );
}

#[test]
fn inherit_directive_is_dropped_from_every_serialized_agent() {
    let fields = agent_fields().expect("catalog decodes");
    let body = serde_json::to_string(&fields).expect("fields serialize");
    assert!(!body.contains("inherit_credentials_from_provider"));
}

#[test]
fn raw_catalog_still_carries_the_inherit_directive_before_merging() {
    let raw = raw_agent_catalog().expect("raw catalog decodes");
    let bedrock_agentcore = raw
        .iter()
        .find(|entry| entry.agent_type == "bedrock_agentcore")
        .expect("bedrock_agentcore is present");
    assert_eq!(
        bedrock_agentcore.inherit_from_provider,
        Some("Bedrock".to_owned())
    );
}

#[test]
fn use_a2a_form_fields_never_appears_on_agent_create_info() {
    let fields = agent_fields().expect("catalog decodes");
    let body = serde_json::to_string(&fields).expect("fields serialize");
    assert!(!body.contains("use_a2a_form_fields"));
}

#[test]
fn raw_agent_catalog_decodes_all_eight_agents_on_every_call() {
    let first = raw_agent_catalog().expect("catalog decodes");
    let second = raw_agent_catalog().expect("catalog decodes");
    assert_eq!(first, second);
    assert_eq!(first.len(), 8);
}

#[tokio::test]
async fn agent_fields_error_responds_with_litellms_unhandled_exception_body() {
    let parse_error =
        serde_json::from_slice::<Vec<RawAgentCreateInfo>>(b"not json").expect_err("not valid JSON");
    let error = AgentFieldsError::CatalogJson(Arc::new(parse_error));
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        bytes.as_ref(),
        br#"{"error":{"message":"Internal server error","type":"internal_server_error"}}"#
    );
}

#[tokio::test]
async fn get_agent_fields_handler_returns_the_full_catalog() {
    let state: AgentFieldsState = Arc::new(agent_fields());
    let response = get_agent_fields(State(state))
        .await
        .expect("catalog decodes");
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(body.contains(r#""agent_type":"a2a""#));
    assert!(body.contains(r#""agent_type":"watsonx_orchestrate""#));
}

#[tokio::test]
async fn agent_fields_path_serves_the_full_catalog_through_a_router() {
    let router = agent_fields_routes();
    let request = Request::get(AGENT_FIELDS_PATH).body(Body::empty()).unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Vec<AgentCreateInfo> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body, agent_fields().expect("catalog decodes"));
}
