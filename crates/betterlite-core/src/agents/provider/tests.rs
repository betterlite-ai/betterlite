#![cfg(test)]

use super::*;
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn provider_records_keep_unknown_fields() {
    let response: AgentCreateResponse = serde_json::from_value(json!({
        "id": "agents/worker",
        "base_agent": "gemini",
        "system_instruction": {"parts": [{"text": "work"}]},
        "base_environment": "prod"
    }))
    .unwrap();
    assert_eq!(response.id.as_deref(), Some("agents/worker"));
    assert_eq!(response.extra.get("base_agent"), Some(&json!("gemini")));

    let encoded = serde_json::to_value(response).unwrap();
    assert_eq!(encoded["base_environment"], "prod");
}

#[test]
fn list_agents_response_wraps_agent_records() {
    let response: ListAgentsResponse = serde_json::from_value(json!({
        "agents": [{"agent_id": "a", "agent_name": "worker", "agent_card_params": {}}]
    }))
    .unwrap();
    assert_eq!(response.agents[0].agent_id, "a");
}

#[test]
fn a2a_error_gets_request_id_and_bool_is_stringified() {
    let request_id = json!(true);
    let response = LiteLLMSendMessageResponse::from_dict(
        &json!({"jsonrpc": "2.0", "error": {"code": -32600}}),
        Some(&request_id),
    )
    .unwrap();
    assert_eq!(response.id, Some(json!("true")));
    assert_eq!(response.jsonrpc, "2.0");
}

#[test]
fn a2a_existing_bool_id_is_stringified() {
    let response = LiteLLMSendMessageResponse::from_dict(
        &json!({"jsonrpc": "2.0", "id": true, "result": {}}),
        None,
    )
    .unwrap();
    assert_eq!(response.id, Some(json!("true")));
}

#[test]
fn default_error_class_preserves_http_context() {
    let error = TestProvider::get_error_class("upstream", 502, &[]);
    assert_eq!(error.status_code(), 502);
    assert_eq!(error.message(), "upstream");
}

struct TestProvider;

impl BaseAgentsAPIConfig for TestProvider {
    fn get_complete_url(
        &self,
        _api_base: Option<&str>,
        _parameters: &BTreeMap<String, AgentJson>,
    ) -> Result<String, AgentProviderError> {
        Ok(String::from("https://example.test/agents"))
    }

    fn validate_environment(
        &self,
        headers: &BTreeMap<String, String>,
        _parameters: &BTreeMap<String, AgentJson>,
    ) -> Result<BTreeMap<String, String>, AgentProviderError> {
        Ok(headers.clone())
    }

    fn transform_create_request(
        &self,
        _name: &str,
        _parameters: &BTreeMap<String, AgentJson>,
    ) -> Result<AgentRequest, AgentProviderError> {
        Ok(AgentRequest::default())
    }

    fn transform_create_response(
        &self,
        response: &AgentHttpResponse,
        _name: &str,
    ) -> Result<AgentCreateResponse, AgentProviderError> {
        response.decode()
    }

    fn transform_list_request(
        &self,
        _api_base: Option<&str>,
        _parameters: &BTreeMap<String, AgentJson>,
    ) -> Result<AgentRequest, AgentProviderError> {
        Ok(AgentRequest::default())
    }

    fn transform_list_response(
        &self,
        response: &AgentHttpResponse,
    ) -> Result<AgentListResponse, AgentProviderError> {
        response.decode()
    }

    fn transform_get_request(
        &self,
        _name: &str,
        _api_base: Option<&str>,
        _parameters: &BTreeMap<String, AgentJson>,
    ) -> Result<AgentRequest, AgentProviderError> {
        Ok(AgentRequest::default())
    }

    fn transform_get_response(
        &self,
        response: &AgentHttpResponse,
        _name: &str,
    ) -> Result<AgentCreateResponse, AgentProviderError> {
        response.decode()
    }

    fn transform_delete_request(
        &self,
        _name: &str,
        _api_base: Option<&str>,
        _parameters: &BTreeMap<String, AgentJson>,
    ) -> Result<AgentRequest, AgentProviderError> {
        Ok(AgentRequest::default())
    }

    fn transform_delete_response(
        &self,
        response: &AgentHttpResponse,
        name: &str,
    ) -> Result<AgentDeleteResult, AgentProviderError> {
        let mut result: AgentDeleteResult = response.decode()?;
        result.name = String::from(name);
        Ok(result)
    }

    fn transform_versions_request(
        &self,
        _name: &str,
        _api_base: Option<&str>,
        _parameters: &BTreeMap<String, AgentJson>,
    ) -> Result<AgentRequest, AgentProviderError> {
        Ok(AgentRequest::default())
    }

    fn transform_versions_response(
        &self,
        response: &AgentHttpResponse,
        _name: &str,
    ) -> Result<AgentVersionsResponse, AgentProviderError> {
        response.decode()
    }
}
