#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::{Map, Value, json};

use crate::api::container_contracts::config::{BaseContainerConfig as _, ContainerListQuery};
use crate::api::container_contracts::error::ContainerError;
use crate::api::container_contracts::records::{
    ContainerFileObjectLiteral, ContainerObjectLiteral, DeletedContainerLiteral, ExpiresAfter,
    ExpiresAfterAnchor,
};
use crate::containers::urls::ContainerResource;
use crate::containers::{costs, urls};

use super::support::{
    AZURE, BASE, CONTAINER_BODY, CONTAINER_NULL_BODY, CONTAINER_WRONG_OBJECT, Context, DELETE_BODY,
    FILE_LIST_BODY, LIST_BODY, RecorderCost, config_with, config_with_costs, fresh_config,
    globals_with, globals_with_base, settings_with,
};

#[derive(Clone, Copy)]
enum Scenario {
    BaseWithV1,
    BaseWithContainers,
    CreateExpiresAfter,
    ListLimitOrder,
    RetrieveDeleteId,
    FileListAndContent,
    ContainerAuthChain,
    ContainerCostHeader,
    ContainerResponseError,
}

#[rstest]
#[case::base_with_v1(Scenario::BaseWithV1)]
#[case::base_with_containers(Scenario::BaseWithContainers)]
#[case::create_expires_after(Scenario::CreateExpiresAfter)]
#[case::list_limit_order(Scenario::ListLimitOrder)]
#[case::retrieve_delete_id(Scenario::RetrieveDeleteId)]
#[case::file_list_and_content(Scenario::FileListAndContent)]
#[case::container_auth_chain(Scenario::ContainerAuthChain)]
#[case::container_cost_header(Scenario::ContainerCostHeader)]
#[case::container_response_error(Scenario::ContainerResponseError)]
fn containers_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::BaseWithV1 => base_with_v1(),
        Scenario::BaseWithContainers => base_with_containers(),
        Scenario::CreateExpiresAfter => create_expires_after(),
        Scenario::ListLimitOrder => list_limit_order(),
        Scenario::RetrieveDeleteId => retrieve_delete_id(),
        Scenario::FileListAndContent => file_list_and_content(),
        Scenario::ContainerAuthChain => container_auth_chain(),
        Scenario::ContainerCostHeader => container_cost_header(),
        Scenario::ContainerResponseError => container_response_error(),
    }
}

// The Azure base keeps its api-version query, and no /v1 segment is ever added by the join
fn base_with_v1() {
    assert_eq!(
        urls::join_container_api_base_path(AZURE, "/cntr_abc/files"),
        "https://example.openai.azure.com/openai/v1/containers/cntr_abc/files?api-version=v1"
    );
    assert_eq!(
        urls::join_container_api_base_path(AZURE, "cntr_abc"),
        "https://example.openai.azure.com/openai/v1/containers/cntr_abc?api-version=v1"
    );
    assert_eq!(
        urls::join_container_api_base_path("https://example.test/v1/", "/containers"),
        "https://example.test/v1/containers"
    );
    assert_eq!(
        urls::join_container_api_base_path("https://example.test/v1#frag", "/containers"),
        "https://example.test/v1/containers#frag"
    );
    assert_eq!(
        urls::join_container_api_base_path("https://example.test", "/containers"),
        "https://example.test/containers"
    );
    // a base that is not an absolute URL has no parsed path, so the join trims its slashes as text
    assert_eq!(
        urls::join_container_api_base_path("api.openai.com/v1/", "/containers"),
        "api.openai.com/v1/containers"
    );
}

fn base_with_containers() {
    let params = Map::new();
    let configured = config_with(
        settings_with("https://env.test/v1"),
        globals_with(None, None),
        None,
    );
    assert_eq!(
        configured.get_complete_url(None, &params).unwrap(),
        "https://env.test/v1/containers"
    );
    let global = config_with(
        settings_with("https://env.test/v1"),
        globals_with_base("https://global.test/v1"),
        None,
    );
    assert_eq!(
        global.get_complete_url(None, &params).unwrap(),
        "https://global.test/v1/containers"
    );
    assert_eq!(
        global
            .get_complete_url(Some("https://direct.test/v1/"), &params)
            .unwrap(),
        "https://direct.test/v1/containers"
    );
    assert_eq!(
        global.get_complete_url(Some(""), &params).unwrap(),
        "https://global.test/v1/containers"
    );
    assert_eq!(
        fresh_config().get_complete_url(None, &params).unwrap(),
        "https://api.openai.com/v1/containers"
    );
    assert_eq!(
        fresh_config()
            .get_complete_url(Some("https://direct.test/v1/containers"), &params)
            .unwrap(),
        "https://direct.test/v1/containers/containers"
    );
    assert_eq!(
        fresh_config()
            .get_complete_url(Some("https://direct.test/v1///"), &params)
            .unwrap(),
        "https://direct.test/v1/containers"
    );
}

fn create_expires_after() {
    let config = fresh_config();
    let context = Context::empty();
    assert_eq!(
        config.get_supported_openai_params(),
        ["name", "expires_after", "file_ids", "extra_headers"]
    );
    let params = json!({
        "expires_after": {"anchor": "last_active_at", "minutes": 20},
        "file_ids": ["file-1", "file-2"],
        "extra_headers": {"x-extra": "yes"},
        "extra_body": {"vendor": "x"},
    });
    let supplied = params.as_object().unwrap();
    assert_eq!(
        Value::Object(config.map_openai_params(supplied, true)),
        params
    );
    assert_eq!(
        Value::Object(config.map_openai_params(supplied, false)),
        params
    );
    let request =
        config.transform_container_create_request("cntr-given", supplied, context.request());
    assert_eq!(request.get("extra_headers"), None);
    assert_eq!(
        request.get("name").and_then(Value::as_str),
        Some("cntr-given")
    );
    assert_eq!(request.get("expires_after"), params.get("expires_after"));
    assert_eq!(request.get("file_ids"), params.get("file_ids"));
    assert_eq!(request.get("extra_body"), params.get("extra_body"));
    let overriding = json!({"name": "cntr-override", "file_ids": ["file-3"]});
    let request = config.transform_container_create_request(
        "cntr-given",
        overriding.as_object().unwrap(),
        context.request(),
    );
    assert_eq!(
        request.get("name").and_then(Value::as_str),
        Some("cntr-override")
    );
}

fn list_limit_order() {
    let config = fresh_config();
    let context = Context::empty();
    let query = ContainerListQuery {
        after: Some("cntr_a"),
        limit: Some(0),
        order: Some(""),
        extra_query: None,
    };
    let (url, params) = config.transform_container_list_request(BASE, context.request(), &query);
    assert_eq!(url, BASE);
    assert_eq!(params.get("after").and_then(Value::as_str), Some("cntr_a"));
    assert_eq!(params.get("limit").and_then(Value::as_str), Some("0"));
    assert_eq!(params.get("order").and_then(Value::as_str), Some(""));
    let empty = ContainerListQuery {
        after: None,
        limit: None,
        order: None,
        extra_query: None,
    };
    let (_url, params) = config.transform_container_list_request(BASE, context.request(), &empty);
    assert!(params.is_empty());
    let extra = json!({"limit": 5, "after": "other", "api-version": "v1"});
    let extra_map = extra.as_object().unwrap();
    let overriding = ContainerListQuery {
        after: Some("cntr_a"),
        limit: Some(2),
        order: None,
        extra_query: Some(extra_map),
    };
    let (_url, params) =
        config.transform_container_list_request(BASE, context.request(), &overriding);
    assert_eq!(params.get("limit"), Some(&json!(5)));
    assert_eq!(params.get("after").and_then(Value::as_str), Some("other"));
    assert_eq!(
        params.get("api-version").and_then(Value::as_str),
        Some("v1")
    );
    let file_query = ContainerListQuery {
        after: Some("cfile_0"),
        limit: Some(1),
        order: Some("asc"),
        extra_query: None,
    };
    let (url, params) = config
        .transform_container_file_list_request("cntr/abc", BASE, context.request(), &file_query)
        .unwrap();
    assert_eq!(url, "https://api.openai.com/v1/containers/cntr%2Fabc/files");
    assert_eq!(params.get("limit").and_then(Value::as_str), Some("1"));
    assert_eq!(params.get("order").and_then(Value::as_str), Some("asc"));
}

fn retrieve_delete_id() {
    let config = fresh_config();
    let context = Context::empty();
    let (url, data) = config
        .transform_container_retrieve_request("cntr_abc", BASE, context.request())
        .unwrap();
    assert_eq!(url, "https://api.openai.com/v1/containers/cntr_abc");
    assert!(data.is_empty());
    let (url, _data) = config
        .transform_container_retrieve_request("cntr/abc d", AZURE, context.request())
        .unwrap();
    assert_eq!(
        url,
        "https://example.openai.azure.com/openai/v1/containers/cntr%2Fabc%20d?api-version=v1"
    );
    let (url, data) = config
        .transform_container_delete_request("cntr_abc", BASE, context.request())
        .unwrap();
    assert_eq!(url, "https://api.openai.com/v1/containers/cntr_abc");
    assert!(data.is_empty());
    for refused in [".", "..", ""] {
        let retrieve = config
            .transform_container_retrieve_request(refused, BASE, context.request())
            .unwrap_err();
        assert!(matches!(retrieve, ContainerError::PathSegment { .. }));
        assert!(retrieve.to_string().contains("container_id"));
        let delete = config
            .transform_container_delete_request(refused, BASE, context.request())
            .unwrap_err();
        assert!(matches!(delete, ContainerError::PathSegment { .. }));
        assert!(delete.to_string().contains("container_id"));
        let error = urls::resource_url(BASE, ContainerResource::Files(refused)).unwrap_err();
        assert!(error.to_string().contains("container_id"));
    }
}

fn file_list_and_content() {
    let config = fresh_config();
    let context = Context::empty();
    let empty = ContainerListQuery {
        after: None,
        limit: None,
        order: None,
        extra_query: None,
    };
    let (url, params) = config
        .transform_container_file_list_request("cntr_abc", BASE, context.request(), &empty)
        .unwrap();
    assert_eq!(url, "https://api.openai.com/v1/containers/cntr_abc/files");
    assert!(params.is_empty());
    let (url, params) = config
        .transform_container_file_content_request("cntr_abc", "cfile_1", AZURE, context.request())
        .unwrap();
    assert_eq!(
        url,
        "https://example.openai.azure.com/openai/v1/containers/cntr_abc/files/cfile_1/content?api-version=v1"
    );
    assert!(params.is_empty());
    let refused_container = config
        .transform_container_file_list_request("..", BASE, context.request(), &empty)
        .unwrap_err();
    assert!(matches!(
        refused_container,
        ContainerError::PathSegment { .. }
    ));
    assert!(refused_container.to_string().contains("container_id"));
    let refused_file = config
        .transform_container_file_content_request("cntr_abc", "", BASE, context.request())
        .unwrap_err();
    assert!(matches!(refused_file, ContainerError::PathSegment { .. }));
    assert!(refused_file.to_string().contains("file_id"));
    let refused_dot = config
        .transform_container_file_content_request(".", "cfile_1", BASE, context.request())
        .unwrap_err();
    assert!(matches!(refused_dot, ContainerError::PathSegment { .. }));
    assert!(refused_dot.to_string().contains("container_id"));
    let bytes = b"\x00raw\xffbytes".to_vec();
    assert_eq!(
        config
            .transform_container_file_content_response(&bytes)
            .unwrap(),
        bytes
    );
    let files = config
        .transform_container_file_list_response(FILE_LIST_BODY.as_bytes())
        .unwrap();
    assert_eq!(files.data.len(), 1);
    assert_eq!(files.data[0].object, ContainerFileObjectLiteral::Dotted);
    assert_eq!(files.data[0].bytes, Some(12));
    assert!(!files.has_more);
}

fn authorization(headers: &BTreeMap<String, String>) -> &str {
    headers
        .get("Authorization")
        .map(String::as_str)
        .unwrap_or_default()
}

fn container_auth_chain() {
    let headers = BTreeMap::new();
    let config = config_with(
        settings_with("https://env.test/v1"),
        globals_with(Some("sk-global"), Some("sk-openai")),
        Some("sk-env"),
    );
    assert_eq!(
        authorization(&config.validate_environment(&headers, Some("sk-explicit"))),
        "Bearer sk-explicit"
    );
    assert_eq!(
        authorization(&config.validate_environment(&headers, None)),
        "Bearer sk-global"
    );
    let openai_only = config_with(
        settings_with("https://env.test/v1"),
        globals_with(None, Some("sk-openai")),
        Some("sk-env"),
    );
    assert_eq!(
        authorization(&openai_only.validate_environment(&headers, None)),
        "Bearer sk-openai"
    );
    let env_only = config_with(
        settings_with("https://env.test/v1"),
        globals_with(None, None),
        Some("sk-env"),
    );
    assert_eq!(
        authorization(&env_only.validate_environment(&headers, None)),
        "Bearer sk-env"
    );
    assert_eq!(
        authorization(&fresh_config().validate_environment(&headers, None)),
        "Bearer None"
    );
    // an empty env var counts as unset, so no key resolves and the header keeps its unresolved form
    let empty_env = config_with(
        settings_with("https://env.test/v1"),
        globals_with(None, None),
        Some(""),
    );
    assert_eq!(
        authorization(&empty_env.validate_environment(&headers, None)),
        "Bearer None"
    );
    assert_eq!(
        authorization(&env_only.validate_environment(&headers, Some(""))),
        "Bearer sk-env"
    );
    let mut supplied = BTreeMap::new();
    supplied.insert("Authorization".to_owned(), "Bearer stale".to_owned());
    supplied.insert("authorization".to_owned(), "kept".to_owned());
    supplied.insert("x-extra".to_owned(), "yes".to_owned());
    let resolved = env_only.validate_environment(&supplied, Some("sk-explicit"));
    assert_eq!(
        resolved.get("Authorization").map(String::as_str),
        Some("Bearer sk-explicit")
    );
    assert_eq!(
        resolved.get("authorization").map(String::as_str),
        Some("kept")
    );
    assert_eq!(resolved.get("x-extra").map(String::as_str), Some("yes"));
    assert_eq!(
        supplied.get("Authorization").map(String::as_str),
        Some("Bearer stale")
    );
}

fn cost_header(hidden: &Map<String, Value>) -> &Value {
    hidden
        .get("additional_headers")
        .and_then(|headers| headers.get("llm_provider-x-litellm-response-cost"))
        .unwrap()
}

fn container_cost_header() {
    let recorder = Arc::new(RecorderCost::default());
    let config = config_with_costs(Arc::<RecorderCost>::clone(&recorder));
    let created = config
        .transform_container_create_response(CONTAINER_BODY.as_bytes())
        .unwrap();
    assert_eq!(recorder.calls(), vec![(1, String::from("openai"))]);
    assert_eq!(cost_header(&created.hidden_params).to_string(), "0.03");
    assert_eq!(created.hidden_params.len(), 1);
    let retrieved = config
        .transform_container_retrieve_response(CONTAINER_BODY.as_bytes())
        .unwrap();
    assert!(retrieved.hidden_params.is_empty());
    let listed = config
        .transform_container_list_response(LIST_BODY.as_bytes())
        .unwrap();
    assert!(
        listed
            .data
            .iter()
            .all(|entry| entry.hidden_params.is_empty())
    );
    let other = costs::cost_hidden_params(Decimal::new(75, 2)).unwrap();
    assert_eq!(cost_header(&other).to_string(), "0.75");
    assert_eq!(other.len(), 1);
    let whole = costs::cost_hidden_params(Decimal::new(300, 2)).unwrap();
    assert_eq!(cost_header(&whole).to_string(), "3");
}

fn container_response_error() {
    let config = fresh_config();
    let container = config
        .transform_container_retrieve_response(CONTAINER_BODY.as_bytes())
        .unwrap();
    assert_eq!(container.id, "cntr_abc");
    assert_eq!(container.object, ContainerObjectLiteral::Container);
    assert_eq!(container.status, "running");
    assert_eq!(container.name.as_deref(), Some("box"));
    assert_eq!(
        container.expires_after,
        Some(ExpiresAfter {
            anchor: ExpiresAfterAnchor::LastActiveAt,
            minutes: 20,
        })
    );
    let nullable = config
        .transform_container_retrieve_response(CONTAINER_NULL_BODY.as_bytes())
        .unwrap();
    assert_eq!(nullable.expires_after, None);
    assert_eq!(nullable.last_active_at, None);
    assert_eq!(nullable.name, None);
    let list = config
        .transform_container_list_response(LIST_BODY.as_bytes())
        .unwrap();
    assert_eq!(list.data.len(), 2);
    assert_eq!(list.first_id.as_deref(), Some("cntr_a"));
    assert_eq!(list.last_id.as_deref(), Some("cntr_b"));
    assert!(list.has_more);
    let deleted = config
        .transform_container_delete_response(DELETE_BODY.as_bytes())
        .unwrap();
    assert_eq!(deleted.object, DeletedContainerLiteral::Container);
    assert!(deleted.deleted);
    let files = config
        .transform_container_file_list_response(FILE_LIST_BODY.as_bytes())
        .unwrap();
    assert_eq!(files.last_id, None);
    let wrong = config
        .transform_container_retrieve_response(CONTAINER_WRONG_OBJECT.as_bytes())
        .unwrap_err();
    assert!(matches!(wrong, ContainerError::Decode(_)), "{wrong:?}");
    let wrong_list = config
        .transform_container_delete_response(CONTAINER_BODY.as_bytes())
        .unwrap_err();
    assert!(
        matches!(wrong_list, ContainerError::Decode(_)),
        "{wrong_list:?}"
    );
    let missing = config
        .transform_container_create_response(b"{}")
        .unwrap_err();
    assert!(matches!(missing, ContainerError::Decode(_)), "{missing:?}");
    // a derived record takes an array in field order, but LiteLLM's model_validate takes only an object
    let array = config
        .transform_container_retrieve_response(
            br#"["cntr_1","container",1,"running",null,null,null]"#,
        )
        .unwrap_err();
    assert!(matches!(array, ContainerError::Decode(_)), "{array:?}");
    let repeated = config
        .transform_container_retrieve_response(
            br#"{"id":"cntr_first","object":"container","created_at":1,"status":"running","id":"cntr_last"}"#,
        )
        .unwrap();
    assert_eq!(repeated.id, "cntr_last");
    assert_eq!(
        config
            .transform_container_file_content_response(b"bytes")
            .unwrap(),
        b"bytes"
    );
    let mut headers = BTreeMap::new();
    headers.insert("retry-after".to_owned(), "3".to_owned());
    let error = config.get_error_class("rate limited", 429, &headers);
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "rate limited");
    assert_eq!(error.headers, headers);
}
