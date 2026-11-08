#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use serde_json::{Map, Value};
use sha2::{Digest as _, Sha256};

use super::catalog::{ContainerEndpoint, container_endpoints};
use super::config::{BaseContainerConfig, ContainerListQuery, ContainerRequestContext};
use super::error::{BaseLlmException, ContainerError};
use super::records::{
    ContainerCreateOptionalParams, ContainerCreateRequestParams, ContainerFileListResponse,
    ContainerFileObject, ContainerFileObjectLiteral, ContainerListOptionalParams,
    ContainerListResponse, ContainerObject, ContainerObjectLiteral, DeleteContainerFileResponse,
    DeleteContainerResult, DeletedContainerFileLiteral, ExpiresAfter, ExpiresAfterAnchor,
    ListObjectLiteral,
};

struct FixtureConfig;

type Prepared = (String, Map<String, Value>);
type Reply<T> = Result<T, ContainerError>;

#[rustfmt::skip]
impl BaseContainerConfig for FixtureConfig {
    fn get_supported_openai_params(&self) -> Vec<String> { Vec::new() }

    fn map_openai_params(&self, _params: &Map<String, Value>, _drop_params: bool) -> Map<String, Value> {
        Map::new()
    }

    fn transform_container_create_request(
        &self, _name: &str, _params: &Map<String, Value>, _context: ContainerRequestContext<'_>,
    ) -> Map<String, Value> { Map::new() }

    fn transform_container_create_response(&self, _raw: &[u8]) -> Reply<ContainerObject> {
        Err(ContainerError::ApiBaseRequired)
    }

    fn transform_container_list_request(
        &self, _base: &str, _context: ContainerRequestContext<'_>, _query: &ContainerListQuery<'_>,
    ) -> Prepared { (String::new(), Map::new()) }

    fn transform_container_list_response(&self, _raw: &[u8]) -> Reply<ContainerListResponse> {
        Err(ContainerError::ApiBaseRequired)
    }

    fn transform_container_retrieve_request(
        &self, _id: &str, _base: &str, _context: ContainerRequestContext<'_>,
    ) -> Prepared { (String::new(), Map::new()) }

    fn transform_container_retrieve_response(&self, _raw: &[u8]) -> Reply<ContainerObject> {
        Err(ContainerError::ApiBaseRequired)
    }

    fn transform_container_delete_request(
        &self, _id: &str, _base: &str, _context: ContainerRequestContext<'_>,
    ) -> Prepared { (String::new(), Map::new()) }

    fn transform_container_delete_response(&self, _raw: &[u8]) -> Reply<DeleteContainerResult> {
        Err(ContainerError::ApiBaseRequired)
    }

    fn transform_container_file_list_request(
        &self, _id: &str, _base: &str, _context: ContainerRequestContext<'_>, _query: &ContainerListQuery<'_>,
    ) -> Prepared { (String::new(), Map::new()) }

    fn transform_container_file_list_response(&self, _raw: &[u8]) -> Reply<ContainerFileListResponse> {
        Err(ContainerError::ApiBaseRequired)
    }

    fn transform_container_file_content_request(
        &self, _id: &str, _file: &str, _base: &str, _context: ContainerRequestContext<'_>,
    ) -> Prepared { (String::new(), Map::new()) }

    fn transform_container_file_content_response(&self, _raw: &[u8]) -> Reply<Vec<u8>> {
        Err(ContainerError::ApiBaseRequired)
    }
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn container_body() -> Value {
    parse(
        r#"{
            "id": "container_abc",
            "object": "container",
            "created_at": 1735689600,
            "status": "running",
            "expires_after": {"anchor": "last_active_at", "minutes": 30},
            "last_active_at": 1735689700,
            "name": "my-container",
            "unknown_key": {"nested": true}
        }"#,
    )
}

fn file_body(literal: &str) -> Value {
    let text = r#"{
        "id": "file_abc",
        "object": "container.file",
        "container_id": "container_abc",
        "bytes": 12,
        "created_at": 1735689600,
        "path": "/mnt/data/report.csv",
        "source": "assistant"
    }"#;
    parse(&text.replace("container.file", literal))
}

#[test]
fn record_container_object_decodes_the_wire_json_and_ignores_unknown_keys() {
    let object: ContainerObject = serde_json::from_value(container_body()).unwrap();
    assert_eq!(object.id, "container_abc");
    assert_eq!(object.object, ContainerObjectLiteral::Container);
    assert_eq!(object.created_at, 1_735_689_600);
    assert_eq!(object.status, "running");
    assert_eq!(
        object.expires_after,
        Some(ExpiresAfter {
            anchor: ExpiresAfterAnchor::LastActiveAt,
            minutes: 30,
        })
    );
    assert_eq!(object.last_active_at, Some(1_735_689_700));
    assert_eq!(object.name.as_deref(), Some("my-container"));
    assert!(object.hidden_params.is_empty());
}

#[test]
fn record_container_object_encodes_every_field_with_nulls_and_omits_hidden_params() {
    let object = ContainerObject {
        id: "container_abc".to_owned(),
        object: ContainerObjectLiteral::Container,
        created_at: 1,
        status: "queued".to_owned(),
        expires_after: None,
        last_active_at: None,
        name: None,
        hidden_params: Map::from_iter([("additional_headers".to_owned(), parse(r#"{"a": "b"}"#))]),
    };
    assert_eq!(
        serde_json::to_value(&object).unwrap(),
        parse(
            r#"{
                "id": "container_abc",
                "object": "container",
                "created_at": 1,
                "status": "queued",
                "expires_after": null,
                "last_active_at": null,
                "name": null
            }"#
        )
    );
}

#[rstest]
#[case::wrong_object_literal(
    r#"{"id": "c", "object": "other", "created_at": 1, "status": "running"}"#
)]
#[case::missing_id(r#"{"object": "container", "created_at": 1, "status": "running"}"#)]
#[case::missing_created_at(r#"{"id": "c", "object": "container", "status": "running"}"#)]
#[case::missing_status(r#"{"id": "c", "object": "container", "created_at": 1}"#)]
#[case::wrong_anchor(r#"{"id": "c", "object": "container", "created_at": 1, "status": "running", "expires_after": {"anchor": "created_at", "minutes": 1}}"#)]
#[case::missing_minutes(r#"{"id": "c", "object": "container", "created_at": 1, "status": "running", "expires_after": {"anchor": "last_active_at"}}"#)]
fn record_container_object_rejects_invalid_bodies(#[case] body: &str) {
    serde_json::from_value::<ContainerObject>(parse(body)).unwrap_err();
}

#[test]
fn record_container_list_response_requires_has_more() {
    serde_json::from_value::<ContainerListResponse>(parse(r#"{"object": "list", "data": []}"#))
        .unwrap_err();
    let response: ContainerListResponse = serde_json::from_value(parse(
        r#"{
            "object": "list",
            "data": [{
                "id": "container_abc",
                "object": "container",
                "created_at": 1,
                "status": "running"
            }],
            "first_id": "container_abc",
            "last_id": "container_abc",
            "has_more": false,
            "unknown_key": 1
        }"#,
    ))
    .unwrap();
    assert_eq!(response.object, ListObjectLiteral::List);
    assert_eq!(response.data.len(), 1);
    assert_eq!(response.first_id.as_deref(), Some("container_abc"));
    assert!(!response.has_more);
}

#[test]
fn record_container_list_response_encodes_absent_ids_as_null() {
    let response = ContainerListResponse {
        object: ListObjectLiteral::List,
        data: Vec::new(),
        first_id: None,
        last_id: None,
        has_more: true,
    };
    assert_eq!(
        serde_json::to_value(&response).unwrap(),
        parse(
            r#"{"object": "list", "data": [], "first_id": null, "last_id": null, "has_more": true}"#
        )
    );
}

#[test]
fn record_delete_result_decodes_the_deleted_literal() {
    let result: DeleteContainerResult = serde_json::from_value(parse(
        r#"{"id": "container_abc", "object": "container.deleted", "deleted": true, "unknown_key": "ignored"}"#,
    ))
    .unwrap();
    assert_eq!(result.id, "container_abc");
    assert!(result.deleted);
    assert_eq!(
        serde_json::to_value(&result).unwrap(),
        parse(r#"{"id": "container_abc", "object": "container.deleted", "deleted": true}"#)
    );
}

#[rstest]
#[case::container_literal(r#"{"id": "c", "object": "container", "deleted": true}"#)]
#[case::missing_deleted(r#"{"id": "c", "object": "container.deleted"}"#)]
fn record_delete_result_rejects_invalid_bodies(#[case] body: &str) {
    serde_json::from_value::<DeleteContainerResult>(parse(body)).unwrap_err();
}

#[rstest]
#[case::dotted("container.file")]
#[case::underscore("container_file")]
fn record_file_object_accepts_both_object_literals(#[case] literal: &str) {
    let file: ContainerFileObject = serde_json::from_value(file_body(literal)).unwrap();
    assert_eq!(file.id, "file_abc");
    assert_eq!(file.container_id, "container_abc");
    assert_eq!(file.bytes, Some(12));
    assert_eq!(file.path, "/mnt/data/report.csv");
    assert_eq!(file.source, "assistant");
    assert!(file.hidden_params.is_empty());
    assert_eq!(serde_json::to_value(&file).unwrap()["object"], literal);
}

#[rstest]
#[case::explicit_null(r#"{"id": "f", "object": "container.file", "container_id": "c", "bytes": null, "created_at": 1, "path": "/a", "source": "user"}"#)]
#[case::absent(r#"{"id": "f", "object": "container.file", "container_id": "c", "created_at": 1, "path": "/a", "source": "user"}"#)]
fn record_file_object_allows_null_and_absent_bytes(#[case] body: &str) {
    let file: ContainerFileObject = serde_json::from_value(parse(body)).unwrap();
    assert_eq!(file.bytes, None);
    assert_eq!(serde_json::to_value(&file).unwrap()["bytes"], Value::Null);
}

#[test]
fn record_file_object_rejects_other_object_literals() {
    serde_json::from_value::<ContainerFileObject>(file_body("container.file.deleted")).unwrap_err();
    serde_json::from_value::<ContainerFileObject>(parse(
        r#"{"id": "f", "object": "container.file", "container_id": "c", "created_at": 1, "path": "/a"}"#,
    ))
    .unwrap_err();
}

fn file_list_body() -> Value {
    parse(
        r#"{
            "object": "list",
            "data": [{
                "id": "file_abc",
                "object": "container_file",
                "container_id": "container_abc",
                "bytes": 12,
                "created_at": 1,
                "path": "/mnt/data/report.csv",
                "source": "assistant"
            }],
            "first_id": "file_abc",
            "last_id": null,
            "has_more": false
        }"#,
    )
}

#[test]
fn record_file_list_response_decodes_file_objects() {
    let response: ContainerFileListResponse = serde_json::from_value(file_list_body()).unwrap();
    assert_eq!(response.data.len(), 1);
    assert_eq!(
        response.data.first().unwrap().object,
        ContainerFileObjectLiteral::Underscore
    );
    assert_eq!(serde_json::to_value(&response).unwrap(), file_list_body());
}

#[rstest]
#[case::dotted("container.file.deleted", DeletedContainerFileLiteral::Dotted)]
#[case::underscore("container_file.deleted", DeletedContainerFileLiteral::Underscore)]
fn record_delete_file_response_accepts_both_object_literals(
    #[case] literal: &str,
    #[case] expected: DeletedContainerFileLiteral,
) {
    let text = r#"{"id": "file_abc", "object": "container.file.deleted", "deleted": true}"#;
    let deleted: DeleteContainerFileResponse =
        serde_json::from_value(parse(&text.replace("container.file.deleted", literal))).unwrap();
    assert_eq!(deleted.object, expected);
    assert_eq!(serde_json::to_value(&deleted).unwrap()["object"], literal);
}

#[test]
fn record_delete_file_response_rejects_other_object_literals() {
    serde_json::from_value::<DeleteContainerFileResponse>(parse(
        r#"{"id": "file_abc", "object": "container.file", "deleted": true}"#,
    ))
    .unwrap_err();
}

#[test]
fn record_field_returns_encoded_values_and_none_for_unknown_names() {
    let object: ContainerObject = serde_json::from_value(parse(
        r#"{"id": "container_abc", "object": "container", "created_at": 1, "status": "running", "name": "kept"}"#,
    ))
    .unwrap();
    assert_eq!(object.field("id"), Some(parse(r#""container_abc""#)));
    assert_eq!(object.field("object"), Some(parse(r#""container""#)));
    assert_eq!(object.field("created_at"), Some(parse("1")));
    assert_eq!(object.field("name"), Some(parse(r#""kept""#)));
    assert_eq!(object.field("expires_after"), Some(Value::Null));
    assert_eq!(object.field("hidden_params"), None);
    assert_eq!(object.field("_hidden_params"), None);
    assert_eq!(object.field("unknown"), None);
    let deleted = DeleteContainerFileResponse {
        id: "file_abc".to_owned(),
        object: DeletedContainerFileLiteral::Dotted,
        deleted: true,
    };
    assert_eq!(
        deleted.field("object"),
        Some(parse(r#""container.file.deleted""#))
    );
    assert_eq!(deleted.field("missing"), None);
}

#[test]
fn params_create_optional_skips_none_fields() {
    let params = ContainerCreateOptionalParams {
        expires_after: None,
        file_ids: None,
        extra_headers: None,
        extra_body: None,
    };
    assert_eq!(serde_json::to_value(&params).unwrap(), parse("{}"));
}

#[test]
fn params_create_optional_keeps_declared_key_names() {
    let params = ContainerCreateOptionalParams {
        expires_after: Some(Map::from_iter([(
            "anchor".to_owned(),
            Value::String("last_active_at".to_owned()),
        )])),
        file_ids: Some(vec!["file_abc".to_owned()]),
        extra_headers: Some(BTreeMap::from([("X-Test".to_owned(), "one".to_owned())])),
        extra_body: Some(BTreeMap::from([("metadata".to_owned(), "{}".to_owned())])),
    };
    assert_eq!(
        serde_json::to_value(&params).unwrap(),
        parse(
            r#"{
                "expires_after": {"anchor": "last_active_at"},
                "file_ids": ["file_abc"],
                "extra_headers": {"X-Test": "one"},
                "extra_body": {"metadata": "{}"}
            }"#
        )
    );
}

#[test]
fn params_create_request_skips_none_fields_and_keeps_name() {
    let params = ContainerCreateRequestParams {
        expires_after: None,
        file_ids: None,
        extra_headers: None,
        extra_body: None,
        name: Some("my-container".to_owned()),
    };
    assert_eq!(
        serde_json::to_value(&params).unwrap(),
        parse(r#"{"name": "my-container"}"#)
    );
    let decoded: ContainerCreateRequestParams = serde_json::from_value(parse(
        r#"{"name": "my-container", "file_ids": ["file_abc"]}"#,
    ))
    .unwrap();
    assert_eq!(decoded.name.as_deref(), Some("my-container"));
    assert_eq!(decoded.file_ids, Some(vec!["file_abc".to_owned()]));
    assert_eq!(decoded.expires_after, None);
}

#[test]
fn params_list_optional_skips_none_fields() {
    let params = ContainerListOptionalParams {
        after: None,
        limit: None,
        order: None,
        extra_headers: None,
        extra_query: None,
    };
    assert_eq!(serde_json::to_value(&params).unwrap(), parse("{}"));
}

#[test]
fn params_list_optional_keeps_declared_key_names() {
    let params = ContainerListOptionalParams {
        after: Some("container_abc".to_owned()),
        limit: Some(0),
        order: Some("asc".to_owned()),
        extra_headers: Some(BTreeMap::from([("X-Test".to_owned(), "one".to_owned())])),
        extra_query: Some(BTreeMap::from([(
            "api-version".to_owned(),
            "v1".to_owned(),
        )])),
    };
    assert_eq!(
        serde_json::to_value(&params).unwrap(),
        parse(
            r#"{
                "after": "container_abc",
                "limit": 0,
                "order": "asc",
                "extra_headers": {"X-Test": "one"},
                "extra_query": {"api-version": "v1"}
            }"#
        )
    );
}

const PINNED_CATALOG: &str = r#"[
    {
        "name": "list_container_files",
        "async_name": "alist_container_files",
        "path": "/containers/{container_id}/files",
        "method": "GET",
        "path_params": ["container_id"],
        "query_params": ["after", "limit", "order"],
        "response_type": "ContainerFileListResponse",
        "is_multipart": false,
        "returns_binary": false
    },
    {
        "name": "upload_container_file",
        "async_name": "aupload_container_file",
        "path": "/containers/{container_id}/files",
        "method": "POST",
        "path_params": ["container_id"],
        "query_params": [],
        "response_type": "ContainerFileObject",
        "is_multipart": true,
        "returns_binary": false
    },
    {
        "name": "retrieve_container_file",
        "async_name": "aretrieve_container_file",
        "path": "/containers/{container_id}/files/{file_id}",
        "method": "GET",
        "path_params": ["container_id", "file_id"],
        "query_params": [],
        "response_type": "ContainerFileObject",
        "is_multipart": false,
        "returns_binary": false
    },
    {
        "name": "delete_container_file",
        "async_name": "adelete_container_file",
        "path": "/containers/{container_id}/files/{file_id}",
        "method": "DELETE",
        "path_params": ["container_id", "file_id"],
        "query_params": [],
        "response_type": "DeleteContainerFileResponse",
        "is_multipart": false,
        "returns_binary": false
    },
    {
        "name": "retrieve_container_file_content",
        "async_name": "aretrieve_container_file_content",
        "path": "/containers/{container_id}/files/{file_id}/content",
        "method": "GET",
        "path_params": ["container_id", "file_id"],
        "query_params": [],
        "response_type": "raw",
        "is_multipart": false,
        "returns_binary": true
    }
]"#;

#[test]
fn catalog_contains_the_five_pinned_endpoints_in_order() {
    let endpoints = container_endpoints().unwrap();
    let names: Vec<&str> = endpoints
        .iter()
        .map(|endpoint| endpoint.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "list_container_files",
            "upload_container_file",
            "retrieve_container_file",
            "delete_container_file",
            "retrieve_container_file_content"
        ]
    );
    let async_names: Vec<&str> = endpoints
        .iter()
        .map(|endpoint| endpoint.async_name.as_str())
        .collect();
    assert_eq!(
        async_names,
        [
            "alist_container_files",
            "aupload_container_file",
            "aretrieve_container_file",
            "adelete_container_file",
            "aretrieve_container_file_content"
        ]
    );
}

#[test]
fn catalog_entries_carry_every_pinned_field() {
    let expected: Vec<ContainerEndpoint> = serde_json::from_str(PINNED_CATALOG).unwrap();
    assert_eq!(container_endpoints().unwrap(), expected);
}

// The pinned bytes include the trailing spaces the upstream file carries, so any reformatting fails this hash
#[test]
fn catalog_embedded_text_is_the_pinned_endpoints_file() {
    let digest = hex::encode(Sha256::digest(include_bytes!("fixtures/endpoints.json")));
    assert_eq!(
        digest,
        "37b78192c365a9e8fc0b83ece9ed766c5ac6bc437c5c8806912abad3a6f5bcdb"
    );
}

#[test]
fn catalog_flags_default_to_false_and_mark_multipart_and_binary_entries() {
    let endpoints = container_endpoints().unwrap();
    let flags: Vec<(&str, bool, bool)> = endpoints
        .iter()
        .map(|endpoint| {
            (
                endpoint.name.as_str(),
                endpoint.is_multipart,
                endpoint.returns_binary,
            )
        })
        .collect();
    assert_eq!(
        flags,
        [
            ("list_container_files", false, false),
            ("upload_container_file", true, false),
            ("retrieve_container_file", false, false),
            ("delete_container_file", false, false),
            ("retrieve_container_file_content", false, true)
        ]
    );
}

#[test]
fn config_default_validate_environment_returns_an_empty_map() {
    let headers = BTreeMap::from([("X-Test".to_owned(), "one".to_owned())]);
    assert!(
        FixtureConfig
            .validate_environment(&headers, Some("key"))
            .is_empty()
    );
}

#[rstest]
#[case::plain("https://api.example.test/v1")]
#[case::empty("")]
#[case::trailing_slash("https://api.example.test/v1/")]
fn config_default_get_complete_url_returns_api_base_unchanged(#[case] api_base: &str) {
    assert_eq!(
        FixtureConfig.get_complete_url(Some(api_base), &Map::new()),
        Ok(api_base.to_owned())
    );
}

#[test]
fn config_default_get_complete_url_requires_api_base() {
    let error = FixtureConfig
        .get_complete_url(None, &Map::new())
        .unwrap_err();
    assert_eq!(error, ContainerError::ApiBaseRequired);
    assert_eq!(error.to_string(), "api_base is required");
}

#[test]
fn config_default_get_error_class_carries_status_message_and_headers() {
    let headers = BTreeMap::from([("Retry-After".to_owned(), "3".to_owned())]);
    let raised = FixtureConfig.get_error_class("bad request", 429, &headers);
    assert_eq!(
        raised,
        BaseLlmException {
            status_code: 429,
            message: "bad request".to_owned(),
            headers: headers.clone(),
        }
    );
    assert_eq!(raised.to_string(), "bad request");
    assert_eq!(
        ContainerError::from(raised),
        ContainerError::Upstream(BaseLlmException {
            status_code: 429,
            message: "bad request".to_owned(),
            headers,
        })
    );
}

#[test]
fn config_default_trait_is_object_safe() {
    let config: Box<dyn BaseContainerConfig> = Box::new(FixtureConfig);
    assert!(config.get_supported_openai_params().is_empty());
    assert!(
        config
            .validate_environment(&BTreeMap::new(), None)
            .is_empty()
    );
    assert!(matches!(
        config.get_complete_url(None, &Map::new()),
        Err(ContainerError::ApiBaseRequired)
    ));
}
