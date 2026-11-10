#![cfg(test)]

use std::time::Duration;

use betterlite_anthropic::credentials::{ANTHROPIC_VERSION, AnthropicSettings};
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use futures_util::future::join;
use http::HeaderMap;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use self::fixtures::{
    API_BASE, CALL_ID, Fixture, SKILL_ID, call, call_error, call_with, extra_headers,
    header_values, hosted_params, kwargs, logging, metadata_with, object, redactor, sample_deleted,
    sample_skill, skill_request, text,
};
use super::call::SkillsCallParams;
use super::error::{SkillAction, SkillsCallError, failure};
use super::principal::user_api_key_auth_from_kwargs;

mod fixtures;

// principal extraction

#[test]
fn principal_extraction_prefers_metadata_over_litellm_metadata() {
    let mut values = IndexMap::new();
    values.insert("metadata".to_owned(), metadata_with(text("from-metadata")));
    values.insert(
        "litellm_metadata".to_owned(),
        metadata_with(text("from-litellm-metadata")),
    );
    assert_eq!(
        user_api_key_auth_from_kwargs(&values),
        Some(&text("from-metadata"))
    );
}

#[test]
fn principal_extraction_falls_back_to_litellm_metadata() {
    let mut values = IndexMap::new();
    values.insert("metadata".to_owned(), object(&[]));
    values.insert(
        "litellm_metadata".to_owned(),
        metadata_with(text("principal")),
    );
    assert_eq!(
        user_api_key_auth_from_kwargs(&values),
        Some(&text("principal"))
    );
}

#[rstest]
#[case::text(text("principal"))]
#[case::empty_object(object(&[]))]
#[case::false_value(Json::Bool(false))]
#[case::empty_list(Json::List(Vec::new()))]
fn user_api_key_auth_accepts_any_non_null_value(#[case] principal: Json) {
    let mut values = IndexMap::new();
    values.insert("metadata".to_owned(), metadata_with(principal.clone()));
    assert_eq!(
        user_api_key_auth_from_kwargs(&values),
        Some(&metadata_with(principal))
    );
}

#[test]
fn user_api_key_auth_null_falls_through_to_litellm_metadata() {
    let mut values = IndexMap::new();
    values.insert("metadata".to_owned(), metadata_with(Json::Null));
    values.insert(
        "litellm_metadata".to_owned(),
        metadata_with(text("litellm-principal")),
    );
    assert_eq!(
        user_api_key_auth_from_kwargs(&values),
        Some(&text("litellm-principal"))
    );
}

#[test]
fn principal_extraction_skips_non_object_metadata() {
    let mut values = IndexMap::new();
    values.insert("metadata".to_owned(), text("not-a-dict"));
    values.insert(
        "litellm_metadata".to_owned(),
        metadata_with(text("principal")),
    );
    assert_eq!(
        user_api_key_auth_from_kwargs(&values),
        Some(&text("principal"))
    );
}

#[test]
fn principal_extraction_returns_none_without_either_key() {
    let mut values = IndexMap::new();
    values.insert("metadata".to_owned(), object(&[]));
    assert_eq!(user_api_key_auth_from_kwargs(&values), None);
    assert_eq!(user_api_key_auth_from_kwargs(&IndexMap::new()), None);
}

// lazy hosted handler initialization

#[tokio::test]
async fn lazy_handler_builds_once_for_many_hosted_calls() {
    let fixture = Fixture::new();
    let values = kwargs();
    let hosted = call_with(hosted_params(), SKILL_ID, &values);
    let service = fixture.service();
    service
        .get_skill(skill_request(&hosted, &mut logging()))
        .await
        .expect("the hosted get succeeds");
    service
        .delete_skill(skill_request(&hosted, &mut logging()))
        .await
        .expect("the hosted delete succeeds");
    service
        .get_skill(skill_request(&hosted, &mut logging()))
        .await
        .expect("the second hosted get succeeds");
    assert_eq!(fixture.builds(), 1);
    assert_eq!(fixture.hosted.calls.lock().expect("the fake").len(), 2);
    assert_eq!(fixture.hosted.deletes.lock().expect("the fake").len(), 1);
}

#[tokio::test]
async fn handler_init_reuses_the_cached_instance() {
    let fixture = Fixture::new();
    let values = kwargs();
    let hosted = call_with(hosted_params(), SKILL_ID, &values);
    let service = fixture.service();
    service
        .get_skill(skill_request(&hosted, &mut logging()))
        .await
        .expect("the hosted get succeeds");
    service
        .get_skill(skill_request(&hosted, &mut logging()))
        .await
        .expect("the second hosted get succeeds");
    assert_eq!(fixture.builds(), 1);
}

#[tokio::test]
async fn lazy_handler_is_not_built_for_external_calls() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .get_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect("the external get succeeds");
    assert_eq!(fixture.builds(), 0);
}

// get_skill routing

#[tokio::test]
async fn get_skill_routing_defaults_to_anthropic() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .get_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].provider, "anthropic");
    drop(requests);
    assert_eq!(fixture.builds(), 0);
}

#[tokio::test]
async fn get_skill_hosted_routes_to_the_hosted_handler() {
    let fixture = Fixture::new();
    let values = kwargs();
    let hosted = call_with(hosted_params(), SKILL_ID, &values);
    fixture
        .service()
        .get_skill(skill_request(&hosted, &mut logging()))
        .await
        .expect("the hosted get succeeds");
    assert!(
        fixture
            .http
            .get_requests
            .lock()
            .expect("the fake")
            .is_empty()
    );
    assert_eq!(fixture.hosted.calls.lock().expect("the fake").len(), 1);
    assert_eq!(fixture.builds(), 1);
}

#[tokio::test]
async fn get_skill_external_dispatches_through_the_http_handler() {
    let fixture = Fixture::new();
    let values = kwargs();
    let params = SkillsCallParams {
        api_key: None,
        api_base: None,
        extra_headers: extra_headers("x-request-id", "request-1"),
        extra_query: None,
        timeout: None,
        custom_llm_provider: Some("anthropic".to_owned()),
    };
    let request = call_with(params, SKILL_ID, &values);
    let skill = fixture
        .service()
        .get_skill(skill_request(&request, &mut logging()))
        .await
        .expect("the external get succeeds");
    assert_eq!(skill, sample_skill());
    assert_eq!(fixture.hosted.calls.lock().expect("the fake").len(), 0);
}

// delete_skill routing

#[tokio::test]
async fn delete_skill_routing_defaults_to_anthropic() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .delete_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect("the external delete succeeds");
    let requests = fixture.http.delete_requests.lock().expect("the fake");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].provider, "anthropic");
}

#[tokio::test]
async fn delete_skill_hosted_routes_to_the_hosted_handler() {
    let fixture = Fixture::new();
    let values = kwargs();
    let hosted = call_with(hosted_params(), SKILL_ID, &values);
    fixture
        .service()
        .delete_skill(skill_request(&hosted, &mut logging()))
        .await
        .expect("the hosted delete succeeds");
    assert!(
        fixture
            .http
            .delete_requests
            .lock()
            .expect("the fake")
            .is_empty()
    );
    assert_eq!(fixture.hosted.deletes.lock().expect("the fake").len(), 1);
}

#[tokio::test]
async fn delete_skill_external_returns_the_deleted_skill() {
    let fixture = Fixture::new();
    let values = kwargs();
    let deleted = fixture
        .service()
        .delete_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect("the external delete succeeds");
    assert_eq!(deleted, sample_deleted());
}

// request transformation

#[tokio::test]
async fn get_skill_request_carries_the_validated_headers() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .get_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
    assert_eq!(
        header_values(&requests[0].headers, "x-api-key"),
        ["sk-ant-test"]
    );
    assert!(
        requests[0]
            .headers
            .get("x-api-key")
            .expect("the auth header exists")
            .is_sensitive()
    );
    assert_eq!(
        header_values(&requests[0].headers, "anthropic-version"),
        [ANTHROPIC_VERSION]
    );
    assert_eq!(
        header_values(&requests[0].headers, "anthropic-beta"),
        ["skills-2025-10-02"]
    );
    assert_eq!(
        header_values(&requests[0].headers, "content-type"),
        ["application/json"]
    );
}

#[tokio::test]
async fn get_skill_request_keeps_caller_extra_headers() {
    let fixture = Fixture::new();
    let values = kwargs();
    let params = SkillsCallParams {
        api_key: None,
        api_base: None,
        extra_headers: extra_headers("anthropic-beta", "custom-beta"),
        extra_query: None,
        timeout: None,
        custom_llm_provider: None,
    };
    let request = call_with(params, SKILL_ID, &values);
    fixture
        .service()
        .get_skill(skill_request(&request, &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
    assert_eq!(
        header_values(&requests[0].headers, "anthropic-beta"),
        ["custom-beta", "skills-2025-10-02"]
    );
}

#[tokio::test]
async fn get_skill_request_reports_a_missing_credential() {
    let mut fixture = Fixture::new();
    fixture.anthropic = AnthropicSettings {
        api_key: None,
        auth_token: None,
        api_base: API_BASE.to_owned(),
        use_bearer_for_custom_base: false,
        anthropic_version: ANTHROPIC_VERSION.to_owned(),
    };
    let values = kwargs();
    let failure = fixture
        .service()
        .get_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect_err("a missing credential fails");
    assert_eq!(
        failure.error.to_string(),
        "ANTHROPIC_API_KEY or ANTHROPIC_AUTH_TOKEN is required for Skills API"
    );
    assert!(
        fixture
            .http
            .get_requests
            .lock()
            .expect("the fake")
            .is_empty()
    );
}

#[tokio::test]
async fn delete_skill_request_carries_the_validated_headers() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .delete_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect("the external delete succeeds");
    let requests = fixture.http.delete_requests.lock().expect("the fake");
    assert_eq!(
        header_values(&requests[0].headers, "x-api-key"),
        ["sk-ant-test"]
    );
    assert_eq!(
        header_values(&requests[0].headers, "anthropic-beta"),
        ["skills-2025-10-02"]
    );
}

#[tokio::test]
async fn transformation_prefers_the_call_api_key() {
    let fixture = Fixture::new();
    let values = kwargs();
    let params = SkillsCallParams {
        api_key: Some(SecretString::from("sk-ant-call")),
        api_base: None,
        extra_headers: HeaderMap::new(),
        extra_query: None,
        timeout: None,
        custom_llm_provider: None,
    };
    let request = call_with(params, SKILL_ID, &values);
    fixture
        .service()
        .get_skill(skill_request(&request, &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
    assert_eq!(
        header_values(&requests[0].headers, "x-api-key"),
        ["sk-ant-call"]
    );
}

// URL construction

#[tokio::test]
async fn get_skill_url_targets_the_skill_endpoint() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .get_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
    assert_eq!(
        requests[0].url,
        "https://api.anthropic.com/v1/skills/skill_1"
    );
}

#[tokio::test]
async fn delete_skill_url_targets_the_skill_endpoint() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .delete_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect("the external delete succeeds");
    let requests = fixture.http.delete_requests.lock().expect("the fake");
    assert_eq!(
        requests[0].url,
        "https://api.anthropic.com/v1/skills/skill_1"
    );
}

#[tokio::test]
async fn url_construction_encodes_the_skill_id() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .get_skill(skill_request(&call("skill/1 ?x", &values), &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
    assert_eq!(
        requests[0].url,
        "https://api.anthropic.com/v1/skills/skill%2F1%20%3Fx"
    );
}

#[tokio::test]
async fn fallback_base_has_a_single_v1_segment() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .get_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
    assert_eq!(
        requests[0].url,
        "https://api.anthropic.com/v1/skills/skill_1"
    );
    assert_eq!(requests[0].url.matches("/v1/").count(), 1);
}

#[tokio::test]
async fn get_skill_url_prefers_the_call_api_base() {
    let fixture = Fixture::new();
    let values = kwargs();
    let params = SkillsCallParams {
        api_key: None,
        api_base: Some("https://gateway.example".to_owned()),
        extra_headers: HeaderMap::new(),
        extra_query: None,
        timeout: None,
        custom_llm_provider: None,
    };
    let request = call_with(params, SKILL_ID, &values);
    fixture
        .service()
        .get_skill(skill_request(&request, &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
    assert_eq!(requests[0].url, "https://gateway.example/v1/skills/skill_1");
}

#[tokio::test]
async fn url_construction_ignores_an_empty_call_api_base() {
    let fixture = Fixture::new();
    let values = kwargs();
    let params = SkillsCallParams {
        api_key: None,
        api_base: Some(String::new()),
        extra_headers: HeaderMap::new(),
        extra_query: None,
        timeout: None,
        custom_llm_provider: None,
    };
    let request = call_with(params, SKILL_ID, &values);
    fixture
        .service()
        .get_skill(skill_request(&request, &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
    assert_eq!(
        requests[0].url,
        "https://api.anthropic.com/v1/skills/skill_1"
    );
}

// pre-call logging

#[tokio::test]
async fn pre_call_logging_records_skill_id_and_provider() {
    let fixture = Fixture::new();
    let values = kwargs();
    let mut state = logging();
    fixture
        .service()
        .get_skill(skill_request(&call(SKILL_ID, &values), &mut state))
        .await
        .expect("the external get succeeds");
    assert_eq!(state.optional_params.get("skill_id"), Some(&text(SKILL_ID)));
    assert_eq!(state.custom_llm_provider.as_deref(), Some("anthropic"));
    assert_eq!(
        state.model_call_details.get("skill_id"),
        Some(&text(SKILL_ID))
    );
}

#[tokio::test]
async fn logging_records_the_call_id_from_the_call() {
    let fixture = Fixture::new();
    let values = kwargs();
    let mut state = logging();
    fixture
        .service()
        .get_skill(skill_request(&call(SKILL_ID, &values), &mut state))
        .await
        .expect("the external get succeeds");
    assert_eq!(
        state.litellm_params.get("litellm_call_id"),
        Some(&text(CALL_ID))
    );
}

#[tokio::test]
async fn pre_call_logging_uses_litellm_proxy_for_hosted() {
    let fixture = Fixture::new();
    let values = kwargs();
    let hosted = call_with(hosted_params(), SKILL_ID, &values);
    let mut state = logging();
    fixture
        .service()
        .get_skill(skill_request(&hosted, &mut state))
        .await
        .expect("the hosted get succeeds");
    assert_eq!(state.custom_llm_provider.as_deref(), Some("litellm_proxy"));
    assert_eq!(state.optional_params.get("skill_id"), Some(&text(SKILL_ID)));
}

// timeout default

#[tokio::test]
async fn timeout_default_uses_the_request_timeout_setting() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .get_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
    assert_eq!(requests[0].timeout, Duration::from_mins(100));
}

#[tokio::test]
async fn timeout_explicit_value_is_preserved() {
    let fixture = Fixture::new();
    let values = kwargs();
    let params = SkillsCallParams {
        api_key: None,
        api_base: None,
        extra_headers: HeaderMap::new(),
        extra_query: None,
        timeout: Some(Duration::from_secs(5)),
        custom_llm_provider: None,
    };
    let request = call_with(params, SKILL_ID, &values);
    fixture
        .service()
        .get_skill(skill_request(&request, &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
    assert_eq!(requests[0].timeout, Duration::from_secs(5));
}

#[tokio::test]
async fn timeout_default_zero_falls_back() {
    let mut fixture = Fixture::new();
    fixture.core.request_timeout_seconds = 30.0_f64;
    let values = kwargs();
    let params = SkillsCallParams {
        api_key: None,
        api_base: None,
        extra_headers: HeaderMap::new(),
        extra_query: None,
        timeout: Some(Duration::ZERO),
        custom_llm_provider: None,
    };
    let request = call_with(params, SKILL_ID, &values);
    fixture
        .service()
        .get_skill(skill_request(&request, &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
    assert_eq!(requests[0].timeout, Duration::from_secs(30));
}

#[tokio::test]
async fn timeout_is_not_consulted_for_hosted_calls() {
    let mut fixture = Fixture::new();
    fixture.core.request_timeout_seconds = -1.0_f64;
    let values = kwargs();
    let hosted = call_with(hosted_params(), SKILL_ID, &values);
    fixture
        .service()
        .get_skill(skill_request(&hosted, &mut logging()))
        .await
        .expect("the hosted get never resolves a timeout");
}

// exception mapping

#[tokio::test]
async fn exception_mapping_maps_a_provider_status() {
    let fixture = Fixture::failing(404);
    let values = kwargs();
    let failure = fixture
        .service()
        .get_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect_err("the provider error fails the call");
    assert!(matches!(failure.class, ErrorClass::NotFound(_)));
    assert_eq!(
        failure.error.to_string(),
        "the provider answered with HTTP 404: the provider refused"
    );
}

#[tokio::test]
async fn exception_mapping_maps_the_delete_path() {
    let fixture = Fixture::failing(401);
    let values = kwargs();
    let failure = fixture
        .service()
        .delete_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect_err("the provider error fails the delete");
    assert!(matches!(failure.class, ErrorClass::Authentication(_)));
}

#[tokio::test]
async fn exception_type_uses_anthropic_mapping() {
    let fixture = Fixture::failing(403);
    let values = kwargs();
    let failure = fixture
        .service()
        .get_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect_err("the provider error fails the call");
    assert!(matches!(failure.class, ErrorClass::PermissionDenied(_)));
}

#[test]
fn exception_type_uses_the_litellm_proxy_family_mapping() {
    let values = kwargs();
    let params = SkillsCallParams {
        api_key: None,
        api_base: None,
        extra_headers: HeaderMap::new(),
        extra_query: None,
        timeout: None,
        custom_llm_provider: Some("litellm_proxy".to_owned()),
    };
    let call = call_with(params, SKILL_ID, &values);
    let mapped = failure(call_error(403), &call, &redactor());
    assert!(matches!(mapped.class, ErrorClass::Api(_)));
}

#[test]
fn exception_type_uses_the_anthropic_class_for_the_same_status() {
    let values = kwargs();
    let call = call(SKILL_ID, &values);
    let mapped = failure(call_error(403), &call, &redactor());
    assert!(matches!(mapped.class, ErrorClass::PermissionDenied(_)));
}

// unsupported provider

#[test]
fn get_skill_error_message_matches_litellm() {
    let error = SkillsCallError::Unsupported {
        action: SkillAction::Get,
        provider: "openai".to_owned(),
    };
    assert_eq!(error.to_string(), "GET skill is not supported for openai");
}

#[test]
fn delete_skill_error_message_matches_litellm() {
    let error = SkillsCallError::Unsupported {
        action: SkillAction::Delete,
        provider: "openai".to_owned(),
    };
    assert_eq!(
        error.to_string(),
        "DELETE skill is not supported for openai"
    );
}

#[tokio::test]
async fn unsupported_provider_reports_the_litellm_message() {
    let fixture = Fixture::new();
    let values = kwargs();
    let params = SkillsCallParams {
        api_key: None,
        api_base: None,
        extra_headers: HeaderMap::new(),
        extra_query: None,
        timeout: None,
        custom_llm_provider: Some("openai".to_owned()),
    };
    let request = call_with(params, SKILL_ID, &values);
    let failure = fixture
        .service()
        .get_skill(skill_request(&request, &mut logging()))
        .await
        .expect_err("the unsupported provider fails the call");
    assert_eq!(
        failure.error.to_string(),
        "GET skill is not supported for openai"
    );
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
}

#[tokio::test]
async fn unsupported_provider_delete_reports_the_litellm_message() {
    let fixture = Fixture::new();
    let values = kwargs();
    let params = SkillsCallParams {
        api_key: None,
        api_base: None,
        extra_headers: HeaderMap::new(),
        extra_query: None,
        timeout: None,
        custom_llm_provider: Some("mistral".to_owned()),
    };
    let request = call_with(params, SKILL_ID, &values);
    let failure = fixture
        .service()
        .delete_skill(skill_request(&request, &mut logging()))
        .await
        .expect_err("the unsupported provider fails the delete");
    assert_eq!(
        failure.error.to_string(),
        "DELETE skill is not supported for mistral"
    );
}

// extra query acceptance

#[tokio::test]
async fn extra_query_is_accepted_and_not_forwarded() {
    let fixture = Fixture::new();
    let values = kwargs();
    let mut extra_query = IndexMap::new();
    extra_query.insert("page".to_owned(), text("2"));
    let params = SkillsCallParams {
        api_key: None,
        api_base: None,
        extra_headers: HeaderMap::new(),
        extra_query: Some(extra_query),
        timeout: None,
        custom_llm_provider: None,
    };
    let request = call_with(params, SKILL_ID, &values);
    fixture
        .service()
        .get_skill(skill_request(&request, &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
    assert_eq!(
        (requests[0].url.as_str(), requests[0].url.contains("page"),),
        ("https://api.anthropic.com/v1/skills/skill_1", false)
    );
}

#[tokio::test]
async fn extra_query_is_not_forwarded_to_the_hosted_handler() {
    let fixture = Fixture::new();
    let values = kwargs();
    let mut extra_query = IndexMap::new();
    extra_query.insert("page".to_owned(), text("2"));
    let params = SkillsCallParams {
        api_key: None,
        api_base: None,
        extra_headers: HeaderMap::new(),
        extra_query: Some(extra_query),
        timeout: None,
        custom_llm_provider: Some("litellm_proxy".to_owned()),
    };
    let hosted = call_with(params, SKILL_ID, &values);
    fixture
        .service()
        .get_skill(skill_request(&hosted, &mut logging()))
        .await
        .expect("the hosted get succeeds");
    let calls = fixture.hosted.calls.lock().expect("the fake");
    assert_eq!(calls[0].skill_id, SKILL_ID);
    assert_eq!(calls[0].litellm_call_id.as_deref(), Some(CALL_ID));
}

// async context and hosted principal

#[tokio::test]
async fn async_context_concurrent_calls_keep_their_own_metadata() {
    let fixture = Fixture::new();
    let first_metadata = object(&[("user", text("first"))]);
    let second_metadata = object(&[("user", text("second"))]);
    let mut first_values = kwargs();
    first_values.insert("metadata".to_owned(), first_metadata.clone());
    let mut second_values = kwargs();
    second_values.insert("metadata".to_owned(), second_metadata.clone());
    let first_call = call(SKILL_ID, &first_values);
    let second_call = call(SKILL_ID, &second_values);
    let service = fixture.service();
    let mut first_state = logging();
    let mut second_state = logging();
    let (first, second) = join(
        service.get_skill(skill_request(&first_call, &mut first_state)),
        service.get_skill(skill_request(&second_call, &mut second_state)),
    )
    .await;
    first.expect("the first call succeeds");
    second.expect("the second call succeeds");
    assert_eq!(
        first_state.litellm_params.get("metadata"),
        Some(&first_metadata)
    );
    assert_eq!(
        second_state.litellm_params.get("metadata"),
        Some(&second_metadata)
    );
}

#[tokio::test]
async fn async_context_hosted_get_receives_the_principal() {
    let fixture = Fixture::new();
    let principal = object(&[("user_id", text("user-1"))]);
    let mut values = kwargs();
    values.insert("metadata".to_owned(), metadata_with(principal.clone()));
    let hosted = call_with(hosted_params(), SKILL_ID, &values);
    fixture
        .service()
        .get_skill(skill_request(&hosted, &mut logging()))
        .await
        .expect("the hosted get succeeds");
    let calls = fixture.hosted.calls.lock().expect("the fake");
    assert_eq!(calls[0].principal, Some(principal));
}
