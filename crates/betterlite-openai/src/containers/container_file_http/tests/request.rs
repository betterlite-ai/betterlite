#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_prompts::media_metadata::{FileBody, FileData};
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::{Map, Value};

use super::support::{BASE, FILE_BODY, LIST_BODY, StubConfig, fixture, handler};
use crate::containers::container_file_http::request::{
    ParamValue, SslVerify, build_query, build_url, parse_method, read_params, request_headers,
};
use crate::containers::container_file_http::transport::ContainerMethod;
use crate::containers::container_file_http::{ContainerFileHttpError, ContainerFileUpload};

fn text(value: &str) -> ParamValue {
    ParamValue::Text(value.to_owned())
}

#[rstest]
#[case::plain(
    "https://h.test/v1/containers",
    "/containers/{container_id}/files",
    "c1",
    "https://h.test/v1/containers/c1/files"
)]
#[case::trailing_slash(
    "https://h.test/v1/containers/",
    "/containers/{container_id}/files",
    "c1",
    "https://h.test/v1/containers/c1/files"
)]
#[case::many_trailing_slashes(
    "https://h.test/v1/containers///",
    "/containers/{container_id}/files",
    "c1",
    "https://h.test/v1/containers/c1/files"
)]
#[case::base_query_kept(
    "https://h.test/openai/v1/containers?api-version=v1",
    "/containers/{container_id}/files",
    "c1",
    "https://h.test/openai/v1/containers/c1/files?api-version=v1"
)]
#[case::slash_in_value(
    "https://h.test/v1/containers",
    "/containers/{container_id}/files",
    "a/b",
    "https://h.test/v1/containers/a%2Fb/files"
)]
#[case::space_and_unicode(
    "https://h.test/v1/containers",
    "/containers/{container_id}/files",
    "a b\u{e9}",
    "https://h.test/v1/containers/a%20b%C3%A9/files"
)]
#[case::unreserved_kept(
    "https://h.test/v1/containers",
    "/containers/{container_id}/files",
    "a-b.c_d~e",
    "https://h.test/v1/containers/a-b.c_d~e/files"
)]
#[case::no_prefix(
    "https://h.test/v1",
    "/{container_id}/files",
    "c1",
    "https://h.test/v1/c1/files"
)]
#[case::encoded_base_path(
    "https://h.test/a%20b/containers",
    "/containers/{container_id}/files",
    "c1",
    "https://h.test/a%20b/containers/c1/files"
)]
fn build_url_joins_the_path_and_keeps_the_query(
    #[case] api_base: &str,
    #[case] template: &str,
    #[case] value: &str,
    #[case] expected: &str,
) {
    let url = build_url(api_base, template, &[("container_id", value.to_owned())]).unwrap();
    assert_eq!(url, expected);
}

#[test]
fn build_url_substitutes_every_declared_parameter() {
    let url = build_url(
        BASE,
        "/containers/{container_id}/files/{file_id}/content",
        &[
            ("container_id", "c1".to_owned()),
            ("file_id", "f 2".to_owned()),
        ],
    )
    .unwrap();
    assert_eq!(url, format!("{BASE}/c1/files/f%202/content"));
}

#[rstest]
#[case::empty("", "container_id is required")]
#[case::dot(".", "container_id cannot be a dot path segment")]
#[case::dot_dot("..", "container_id cannot be a dot path segment")]
fn build_url_rejects_a_value_that_cannot_be_a_segment(#[case] value: &str, #[case] message: &str) {
    let error = build_url(
        BASE,
        "/containers/{container_id}",
        &[("container_id", value.to_owned())],
    )
    .unwrap_err();
    assert!(matches!(error, ContainerFileHttpError::PathSegment(_)));
    assert_eq!(error.to_string(), message);
}

#[test]
fn build_url_rejects_a_base_that_is_not_a_url() {
    let error = build_url("not a url", "/containers", &[]).unwrap_err();
    assert!(matches!(error, ContainerFileHttpError::BaseUrl { url, .. } if url == "not a url"));
}

#[test]
fn handle_treats_a_missing_path_value_as_empty_and_names_the_field() {
    let handler = handler(200, FILE_BODY);
    let mut fixture = fixture();
    fixture.values.remove("file_id");
    let error = handler
        .handle(&fixture.call("retrieve_container_file"))
        .unwrap_err();
    assert_eq!(error.to_string(), "file_id is required");
    assert!(handler.clients.client.seen().is_empty());
}

#[test]
fn handle_encodes_an_integer_path_value_as_its_digits() {
    let handler = handler(200, FILE_BODY);
    let mut fixture = fixture();
    fixture
        .values
        .insert("file_id".to_owned(), ParamValue::Integer(42));
    handler
        .handle(&fixture.call("retrieve_container_file"))
        .unwrap();
    let requests = handler.clients.client.seen();
    assert_eq!(
        requests.first().unwrap().url,
        format!("{BASE}/cntr_1/files/42")
    );
}

#[test]
fn build_query_takes_declared_names_in_catalog_order_and_skips_absent_ones() {
    let values = BTreeMap::from([
        ("order".to_owned(), text("desc")),
        ("limit".to_owned(), ParamValue::Integer(0)),
        ("unrelated".to_owned(), text("x")),
    ]);
    let names = ["after", "limit", "order"].map(str::to_owned);
    let query = build_query(&names, &values, None);
    assert_eq!(
        query.into_iter().collect::<Vec<_>>(),
        [
            ("limit".to_owned(), "0".to_owned()),
            ("order".to_owned(), "desc".to_owned())
        ]
    );
}

#[test]
fn build_query_lets_the_overlay_replace_in_place_and_append() {
    let values = BTreeMap::from([
        ("after".to_owned(), text("a")),
        ("limit".to_owned(), ParamValue::Integer(5)),
    ]);
    let names = ["after", "limit"].map(str::to_owned);
    let overlay = IndexMap::from([
        ("after".to_owned(), text("b")),
        ("extra".to_owned(), ParamValue::Integer(7)),
    ]);
    let query = build_query(&names, &values, Some(&overlay));
    assert_eq!(
        query.into_iter().collect::<Vec<_>>(),
        [
            ("after".to_owned(), "b".to_owned()),
            ("limit".to_owned(), "5".to_owned()),
            ("extra".to_owned(), "7".to_owned())
        ]
    );
}

#[test]
fn handle_sends_no_params_when_the_query_is_empty_so_the_base_query_stays() {
    let handler = handler(200, LIST_BODY);
    let mut fixture = fixture();
    fixture.config = StubConfig::with_base("https://h.test/openai/v1/containers?api-version=v1");
    handler
        .handle(&fixture.call("list_container_files"))
        .unwrap();
    let requests = handler.clients.client.seen();
    let request = requests.first().unwrap();
    assert!(request.query.is_empty());
    assert_eq!(
        request.url,
        "https://h.test/openai/v1/containers/cntr_1/files?api-version=v1"
    );
}

#[test]
fn handle_sends_the_overlay_query_next_to_the_declared_values() {
    let handler = handler(200, LIST_BODY);
    let mut fixture = fixture();
    fixture
        .values
        .insert("limit".to_owned(), ParamValue::Integer(2));
    let overlay = IndexMap::from([("include".to_owned(), text("x"))]);
    let mut call = fixture.call("list_container_files");
    call.extra_query = Some(&overlay);
    handler.handle(&call).unwrap();
    let requests = handler.clients.client.seen();
    assert_eq!(
        requests.first().unwrap().query,
        [
            ("limit".to_owned(), "2".to_owned()),
            ("include".to_owned(), "x".to_owned())
        ]
    );
}

#[test]
fn request_headers_overlay_the_extra_headers_after_validation() {
    let config = StubConfig::with_base(BASE);
    let extra = BTreeMap::from([
        ("Authorization".to_owned(), "Bearer override".to_owned()),
        ("x-trace".to_owned(), "1".to_owned()),
    ]);
    let headers = request_headers(&config, Some(&extra), Some("sk-1"));
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer override")
    );
    assert_eq!(headers.get("x-trace").map(String::as_str), Some("1"));
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    let seen = config.environment.lock().unwrap();
    assert_eq!(*seen, [(extra, Some("sk-1".to_owned()))]);
}

#[test]
fn request_headers_validate_with_an_empty_map_when_there_are_no_extras() {
    let config = StubConfig::with_base(BASE);
    let headers = request_headers(&config, None, None);
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer none")
    );
    assert_eq!(
        *config.environment.lock().unwrap(),
        [(BTreeMap::new(), None)]
    );
}

#[test]
fn handle_asks_the_config_for_the_base_with_the_api_base_and_every_param() {
    let handler = handler(200, LIST_BODY);
    let mut fixture = fixture();
    fixture.params.insert(
        "api_base".to_owned(),
        Value::String("https://given.test".to_owned()),
    );
    fixture
        .params
        .insert("api_key".to_owned(), Value::String("sk-9".to_owned()));
    handler
        .handle(&fixture.call("list_container_files"))
        .unwrap();
    let bases = fixture.config.bases.lock().unwrap();
    assert_eq!(
        *bases,
        [(
            Some("https://given.test".to_owned()),
            fixture.params.clone()
        )]
    );
    assert_eq!(
        fixture
            .config
            .environment
            .lock()
            .unwrap()
            .first()
            .unwrap()
            .1,
        Some("sk-9".to_owned())
    );
}

#[rstest]
#[case::api_key("api_key", Value::Bool(true), "api_key")]
#[case::api_base("api_base", Value::from(3_i64), "api_base")]
#[case::ssl_verify("ssl_verify", Value::from(1_i64), "ssl_verify")]
fn read_params_names_the_field_with_a_wrong_type(
    #[case] field: &str,
    #[case] value: Value,
    #[case] named: &str,
) {
    let params = Map::from_iter([(field.to_owned(), value)]);
    let error = read_params(&params).unwrap_err();
    assert!(error.to_string().contains(named), "{error}");
    assert!(matches!(error, ContainerFileHttpError::ParamType { .. }));
}

#[test]
fn read_params_reads_the_declared_fields() {
    let params = Map::from_iter([
        ("api_key".to_owned(), Value::String("k".to_owned())),
        ("api_base".to_owned(), Value::Null),
        ("ssl_verify".to_owned(), Value::Bool(false)),
    ]);
    let read = read_params(&params).unwrap();
    assert_eq!(read.api_key, Some("k"));
    assert_eq!(read.api_base, None);
    assert_eq!(read.ssl_verify, Some(SslVerify::Enabled(false)));
}

#[rstest]
#[case::get("GET", Some(ContainerMethod::Get))]
#[case::lower_get("get", Some(ContainerMethod::Get))]
#[case::delete("DELETE", Some(ContainerMethod::Delete))]
#[case::post("Post", Some(ContainerMethod::Post))]
#[case::patch("PATCH", None)]
#[case::empty("", None)]
fn parse_method_accepts_get_delete_and_post_in_any_case(
    #[case] method: &str,
    #[case] expected: Option<ContainerMethod>,
) {
    match (parse_method(method), expected) {
        (Ok(parsed), Some(wanted)) => assert_eq!(parsed, wanted),
        (Err(error), None) => assert_eq!(
            error.to_string(),
            format!("Unsupported HTTP method: {}", method.to_ascii_uppercase())
        ),
        (outcome, wanted) => panic!("{outcome:?} does not match {wanted:?}"),
    }
}

fn upload(
    filename: Option<&str>,
    content_type: Option<&str>,
    content: &[u8],
) -> ContainerFileUpload {
    ContainerFileUpload {
        file_data: FileData::Tuple3 {
            filename: filename.map(str::to_owned),
            content: FileBody::Bytes(content.to_vec()),
            content_type: content_type.map(str::to_owned),
        },
    }
}

#[test]
fn handle_uploads_the_file_and_drops_both_content_type_headers() {
    let handler = handler(200, FILE_BODY);
    let mut fixture = fixture();
    fixture.values.remove("file_id");
    let file = upload(Some("report.txt"), Some("text/plain"), b"hello");
    let extra = BTreeMap::from([("CONTENT-TYPE".to_owned(), "keep/me".to_owned())]);
    let mut call = fixture.call("upload_container_file");
    call.file = Some(&file);
    call.extra_headers = Some(&extra);
    handler.handle(&call).unwrap();
    let requests = handler.clients.client.seen();
    let request = requests.first().unwrap();
    assert_eq!(request.method, ContainerMethod::Post);
    let sent = request.file.as_ref().unwrap();
    assert_eq!(sent.filename, "report.txt");
    assert_eq!(sent.content, b"hello");
    assert_eq!(sent.content_type, "text/plain");
    assert!(!request.headers.contains_key("Content-Type"));
    assert!(!request.headers.contains_key("content-type"));
    assert_eq!(
        request.headers.get("CONTENT-TYPE").map(String::as_str),
        Some("keep/me")
    );
    assert_eq!(
        request.headers.get("Authorization").map(String::as_str),
        Some("Bearer none")
    );
}

#[test]
fn handle_logs_the_upload_headers_before_the_content_type_is_removed() {
    let handler = handler(200, FILE_BODY);
    let fixture = fixture();
    let file = upload(Some("a.txt"), None, b"x");
    let mut call = fixture.call("upload_container_file");
    call.file = Some(&file);
    handler.handle(&call).unwrap();
    let seen = handler.logging.seen.lock().unwrap();
    let headers = seen.first().unwrap().1.headers.as_ref().unwrap();
    assert!(headers.contains_key("Content-Type"));
    assert!(headers.contains_key("content-type"));
}

#[rstest]
#[case::named_and_typed(Some("a.csv"), Some("text/csv"), "a.csv", "text/csv")]
#[case::guessed_from_name(Some("a.csv"), None, "a.csv", "text/csv")]
#[case::unnamed(None, None, "file", "application/octet-stream")]
#[case::unnamed_typed(None, Some("image/png"), "file", "image/png")]
fn handle_names_and_types_the_upload_when_the_input_leaves_it_out(
    #[case] filename: Option<&str>,
    #[case] content_type: Option<&str>,
    #[case] expected_name: &str,
    #[case] expected_type: &str,
) {
    let handler = handler(200, FILE_BODY);
    let fixture = fixture();
    let file = upload(filename, content_type, b"x");
    let mut call = fixture.call("upload_container_file");
    call.file = Some(&file);
    handler.handle(&call).unwrap();
    let requests = handler.clients.client.seen();
    let sent = requests.first().unwrap().file.clone().unwrap();
    assert_eq!(sent.filename, expected_name);
    assert_eq!(sent.content_type, expected_type);
}

#[test]
fn handle_posts_without_a_file_when_none_is_given() {
    let handler = handler(200, FILE_BODY);
    let fixture = fixture();
    handler
        .handle(&fixture.call("upload_container_file"))
        .unwrap();
    let requests = handler.clients.client.seen();
    let request = requests.first().unwrap();
    assert!(request.file.is_none());
    assert!(request.headers.contains_key("Content-Type"));
}

#[test]
fn handle_ignores_a_file_on_an_endpoint_that_is_not_multipart() {
    let handler = handler(200, FILE_BODY);
    let fixture = fixture();
    let file = upload(Some("a.txt"), None, b"x");
    let mut call = fixture.call("retrieve_container_file");
    call.file = Some(&file);
    handler.handle(&call).unwrap();
    let requests = handler.clients.client.seen();
    let request = requests.first().unwrap();
    assert!(request.file.is_none());
    assert!(request.headers.contains_key("Content-Type"));
}

#[test]
fn handle_refuses_a_bare_string_as_file_content() {
    let handler = handler(200, FILE_BODY);
    let fixture = fixture();
    let file = ContainerFileUpload {
        file_data: FileData::Direct(FileBody::BareString("/etc/passwd".to_owned())),
    };
    let mut call = fixture.call("upload_container_file");
    call.file = Some(&file);
    let error = handler.handle(&call).unwrap_err();
    assert!(matches!(error, ContainerFileHttpError::Upload(_)));
    assert!(handler.clients.client.seen().is_empty());
}
