#![cfg(test)]

use super::a2a::LiteLLMSendMessageResponse;
use super::{
    AgentCreateResponse, AgentDeleteResult, AgentHttpResponse, AgentListResponse,
    AgentProviderError, AgentRequest, AgentVersionsResponse, BaseAgentsAPIConfig,
    ListAgentsResponse,
};
use crate::agents::json::AgentJson;
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
    assert_eq!(
        response.id,
        Some(super::a2a::JsonRpcId::String(String::from("True")))
    );
    assert_eq!(response.jsonrpc.0, "2.0");
}

#[test]
fn a2a_existing_bool_id_is_stringified() {
    let response = LiteLLMSendMessageResponse::from_dict(
        &json!({"jsonrpc": "2.0", "id": true, "result": {}}),
        None,
    )
    .unwrap();
    assert_eq!(
        response.id,
        Some(super::a2a::JsonRpcId::String(String::from("True")))
    );
}

#[test]
fn default_error_class_preserves_http_context() {
    let error = TestProvider.get_error_class("upstream", 502, &BTreeMap::new());
    assert_eq!(error.status_code(), 502);
    assert_eq!(error.message(), "upstream");
}

struct TestProvider;

macro_rules! provider_fixture {
    ($provider:ty, $($override:tt)*) => {
impl BaseAgentsAPIConfig for $provider {
    type Error = AgentProviderError;
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
    $($override)*
}
    };
}

provider_fixture!(TestProvider,);

struct CustomProvider;
provider_fixture!(
    CustomProvider,
    fn get_error_class(
        &self,
        message: &str,
        _status_code: u16,
        _headers: &BTreeMap<String, String>,
    ) -> Self::Error {
        AgentProviderError::InvalidResponse {
            message: format!("provider: {message}"),
        }
    }
);

#[test]
fn a2a_defaults_preserve_null_fields() {
    let response = LiteLLMSendMessageResponse::from_dict(&json!({}), None).unwrap();
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({
            "id": null, "jsonrpc": "2.0", "result": null, "error": null, "usage": null
        })
    );
}

#[test]
fn versions_use_model_field_name() {
    let payload = json!({"agent_versions": [{"name": "agents/a/versions/v1"}],
        "next_page_token": null, "agentVersions": [{"provider": true}]});
    let response: AgentVersionsResponse = serde_json::from_value(payload.clone()).unwrap();
    assert_eq!(response.agent_versions.len(), 1);
    assert_eq!(response.agent_versions[0]["name"], "agents/a/versions/v1");
    assert_eq!(serde_json::to_value(response).unwrap(), payload);
}

#[test]
fn deletion_requires_name() {
    assert!(serde_json::from_value::<AgentDeleteResult>(json!({})).is_err());
}

#[test]
fn a2a_fractional_response_id_is_rejected() {
    let payload = serde_json::from_str(r#"{"id": 1.5}"#).unwrap();
    assert!(LiteLLMSendMessageResponse::from_dict(&payload, None).is_err());
}

#[rstest::rstest]
#[case::create(json!({"id":null,"name":null,"custom":{"x":[true,null]}}), "create")]
#[case::delete(json!({"name":"a","deleted":true,"custom":{"x":[true,null]}}), "delete")]
#[case::list(json!({"agents":[{"vendor":42}],"next_page_token":"page","custom":null}), "list")]
#[case::versions(json!({"agent_versions":[{"vendor":42}],"next_page_token":"page","custom":null}), "versions")]
fn all_records_preserve_extras(#[case] payload: AgentJson, #[case] kind: &str) {
    let output = match kind {
        "create" => serde_json::to_value(
            serde_json::from_value::<AgentCreateResponse>(payload.clone()).unwrap(),
        ),
        "delete" => serde_json::to_value(
            serde_json::from_value::<AgentDeleteResult>(payload.clone()).unwrap(),
        ),
        "list" => serde_json::to_value(
            serde_json::from_value::<AgentListResponse>(payload.clone()).unwrap(),
        ),
        "versions" => serde_json::to_value(
            serde_json::from_value::<AgentVersionsResponse>(payload.clone()).unwrap(),
        ),
        _ => panic!("invalid fixture"),
    }
    .unwrap();
    assert_eq!(output, payload);
}

#[test]
fn record_defaults_match_litellm() {
    let create: AgentCreateResponse = serde_json::from_str("{}").unwrap();
    let delete: AgentDeleteResult = serde_json::from_str(r#"{"name":"a"}"#).unwrap();
    let list: AgentListResponse = serde_json::from_str("{}").unwrap();
    let versions: AgentVersionsResponse = serde_json::from_str("{}").unwrap();
    assert_eq!(
        serde_json::to_value(create).unwrap(),
        json!({"id":null,"name":null})
    );
    assert_eq!(
        serde_json::to_value(delete).unwrap(),
        json!({"name":"a","deleted":true})
    );
    assert_eq!(
        serde_json::to_value(list).unwrap(),
        json!({"agents":[],"next_page_token":null})
    );
    assert_eq!(
        serde_json::to_value(versions).unwrap(),
        json!({"agent_versions":[],"next_page_token":null})
    );
}

#[rstest::rstest]
#[case::false_id(json!(false), json!("False"))]
#[case::true_id(json!(true), json!("True"))]
#[case::string_id(json!("request"), json!("request"))]
#[case::zero_id(json!(0), json!(0))]
#[case::negative_id(json!(-42), json!(-42))]
#[case::max_unsigned(json!(u64::MAX), json!(u64::MAX))]
#[case::null_id(json!(null), json!(null))]
#[case::list_id(json!([true, null, "hello"]), json!("[True, None, 'hello']"))]
#[case::object_id(json!({"a": false}), json!("{'a': False}"))]
fn request_id_normalization(#[case] request: AgentJson, #[case] expected: AgentJson) {
    let payload = json!({"error":{"code":-32600},"id":null});
    let direct = LiteLLMSendMessageResponse::from_dict(&payload, Some(&request)).unwrap();
    let sdk = LiteLLMSendMessageResponse::from_a2a_response(&payload, Some(&request)).unwrap();
    assert_eq!(serde_json::to_value(&direct).unwrap()["id"], expected);
    assert_eq!(
        serde_json::to_value(&direct).unwrap(),
        serde_json::to_value(sdk).unwrap()
    );
    assert_eq!(payload["id"], json!(null));
}

#[test]
fn a2a_preserves_existing_id_and_metadata() {
    let payload = json!({"id":42,"jsonrpc":"2.0","result":{"text":"answer"},
        "error":null,"usage":{"cost":"0.001"},"vendor":[1,2]});
    let mut response = LiteLLMSendMessageResponse::from_dict(&payload, Some(&json!(99))).unwrap();
    assert!(response.hidden_params.is_empty());
    response
        .hidden_params
        .insert(String::from("cost"), json!("0.001"));
    assert_eq!(serde_json::to_value(response).unwrap(), payload);
}

#[rstest::rstest]
#[case::array(json!([]))]
#[case::null(json!(null))]
#[case::array_id(json!({"id":[]}))]
#[case::object_id(json!({"id":{}}))]
#[case::invalid_result(json!({"result":[]}))]
#[case::invalid_error(json!({"error":"oops"}))]
fn invalid_a2a_records_fail(#[case] payload: AgentJson) {
    assert!(LiteLLMSendMessageResponse::from_dict(&payload, None).is_err());
}

#[rstest::rstest]
#[case::fractional("1.5", "1.5")]
#[case::integer_float("1.0", "1.0")]
fn fractional_request_ids_become_strings(#[case] input: &str, #[case] expected: &str) {
    let request: AgentJson = serde_json::from_str(input).unwrap();
    let response = LiteLLMSendMessageResponse::from_dict(&json!({}), Some(&request)).unwrap();
    assert_eq!(serde_json::to_value(response).unwrap()["id"], expected);
}

#[rstest::rstest]
#[case::client_error(400)]
#[case::authentication(401)]
#[case::missing(404)]
#[case::limited(429)]
#[case::server_error(503)]
fn default_error_keeps_status_and_headers(#[case] status: u16) {
    let headers = BTreeMap::from([(String::from("retry-after"), String::from("5"))]);
    let error = TestProvider.get_error_class("failure", status, &headers);
    assert_eq!(error.status_code(), status);
    assert_eq!(error.message(), "failure");
    assert!(matches!(error, AgentProviderError::Http { headers: actual, .. } if actual == headers));
}

#[test]
fn response_decoding_keeps_error_cause() {
    use std::error::Error;
    let response = AgentHttpResponse {
        status_code: 200,
        body: json!({"agents":false}),
        headers: BTreeMap::new(),
    };
    let error = response.decode::<AgentListResponse>().unwrap_err();
    assert!(error.source().is_some());
    assert_eq!(error.status_code(), 500);
    assert!(error.message().contains("sequence"));
}

#[test]
fn http_failure_preserves_context() {
    let response = AgentHttpResponse {
        status_code: 429,
        body: json!({"error":"slow down"}),
        headers: BTreeMap::from([(String::from("retry-after"), String::from("5"))]),
    };
    let error = response.decode::<AgentListResponse>().unwrap_err();
    assert_eq!(error.status_code(), 429);
    assert_eq!(error.message(), response.body.to_string());
    assert!(
        matches!(error, AgentProviderError::Http { headers, .. } if headers == response.headers)
    );
}

#[test]
fn replays_agent_provider_corpus() {
    let folder =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/agent_provider");
    let entries = std::fs::read_dir(folder).unwrap();
    let count = entries
        .map(|entry| {
            let bytes = std::fs::read(entry.unwrap().path()).unwrap();
            drop(serde_json::from_slice::<AgentCreateResponse>(&bytes));
            drop(serde_json::from_slice::<AgentDeleteResult>(&bytes));
            drop(serde_json::from_slice::<AgentListResponse>(&bytes));
            drop(serde_json::from_slice::<AgentVersionsResponse>(&bytes));
            drop(serde_json::from_slice::<ListAgentsResponse>(&bytes));
            drop(serde_json::from_slice::<LiteLLMSendMessageResponse>(&bytes));
            let payload: AgentJson = serde_json::from_slice(&bytes).unwrap();
            drop(LiteLLMSendMessageResponse::from_dict(
                &payload,
                Some(&payload),
            ));
            drop(LiteLLMSendMessageResponse::from_a2a_response(
                &payload, None,
            ));
        })
        .count();
    assert!(count >= 3);
}

#[test]
fn provider_can_override_error_mapping() {
    let error = CustomProvider.get_error_class("rejected", 400, &BTreeMap::new());
    assert!(matches!(error, AgentProviderError::InvalidResponse { .. }));
    assert_eq!(error.message(), "provider: rejected");
}

#[test]
fn native_contract_exposes_all_operations() {
    let provider = TestProvider;
    let params = BTreeMap::new();
    let headers = BTreeMap::from([(String::from("authorization"), String::from("key"))]);
    assert_eq!(
        provider.get_complete_url(None, &params).unwrap(),
        "https://example.test/agents"
    );
    assert_eq!(
        provider.validate_environment(&headers, &params).unwrap(),
        headers
    );
    assert!(
        provider
            .transform_create_request("a", &params)
            .unwrap()
            .url
            .is_none()
    );
    assert!(
        provider
            .transform_list_request(None, &params)
            .unwrap()
            .body
            .is_none()
    );
    assert!(
        provider
            .transform_get_request("a", None, &params)
            .unwrap()
            .query_params
            .is_empty()
    );
    assert!(
        provider
            .transform_delete_request("a", None, &params)
            .unwrap()
            .headers
            .is_empty()
    );
    assert!(
        provider
            .transform_versions_request("a", None, &params)
            .unwrap()
            .url
            .is_none()
    );
    let raw = AgentHttpResponse {
        status_code: 200,
        body: json!({"id":"a","name":"a"}),
        headers: BTreeMap::new(),
    };
    assert_eq!(
        provider
            .transform_create_response(&raw, "a")
            .unwrap()
            .id
            .as_deref(),
        Some("a")
    );
    assert_eq!(
        provider
            .transform_get_response(&raw, "a")
            .unwrap()
            .name
            .as_deref(),
        Some("a")
    );
    assert_eq!(
        provider
            .transform_delete_response(&raw, "deleted")
            .unwrap()
            .name,
        "deleted"
    );
    assert!(
        provider
            .transform_list_response(&raw)
            .unwrap()
            .agents
            .is_empty()
    );
    assert!(
        provider
            .transform_versions_response(&raw, "a")
            .unwrap()
            .agent_versions
            .is_empty()
    );
}

#[test]
fn arbitrary_integer_ids_keep_precision() {
    let request: AgentJson =
        serde_json::from_str("1234567890123456789012345678901234567890").unwrap();
    let response = LiteLLMSendMessageResponse::from_dict(&json!({}), Some(&request)).unwrap();
    assert_eq!(serde_json::to_value(response).unwrap()["id"], request);
    let existing = json!({"id":request});
    assert!(LiteLLMSendMessageResponse::from_dict(&existing, None).is_ok());
}

#[rstest::rstest]
#[case::apostrophe(json!(["can't"]), "[\"can't\"]")]
#[case::quotes(json!(["say 'hi' and \"bye\""]), "['say \\'hi\\' and \"bye\"']")]
#[case::controls(json!(["\n\r\t\u{0000}\\"]), "['\\n\\r\\t\\x00\\\\']")]
fn compound_request_ids_use_python_spelling(#[case] request: AgentJson, #[case] expected: &str) {
    let response = LiteLLMSendMessageResponse::from_dict(&json!({}), Some(&request)).unwrap();
    assert_eq!(serde_json::to_value(response).unwrap()["id"], expected);
}

#[test]
fn private_tracking_is_local_to_each_record() {
    let mut first: AgentCreateResponse = serde_json::from_str("{}").unwrap();
    let second: AgentCreateResponse = serde_json::from_str("{}").unwrap();
    first
        .hidden_params
        .insert(String::from("cost"), json!("0.01"));
    assert!(second.hidden_params.is_empty());
    assert_eq!(
        serde_json::to_value(first).unwrap(),
        json!({"id":null,"name":null})
    );
    let mut deleted: AgentDeleteResult =
        serde_json::from_str(r#"{"name":"a","deleted":false}"#).unwrap();
    deleted
        .hidden_params
        .insert(String::from("cost"), json!("0.01"));
    assert_eq!(
        serde_json::to_value(deleted).unwrap(),
        json!({"name":"a","deleted":false})
    );
    let mut list = AgentListResponse::default();
    let mut versions = AgentVersionsResponse::default();
    list.hidden_params
        .insert(String::from("cost"), json!("0.01"));
    versions
        .hidden_params
        .insert(String::from("cost"), json!("0.01"));
    assert_eq!(
        serde_json::to_value(list).unwrap(),
        json!({"agents":[],"next_page_token":null})
    );
    assert_eq!(
        serde_json::to_value(versions).unwrap(),
        json!({"agent_versions":[],"next_page_token":null})
    );
}
