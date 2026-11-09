#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use super::create::create_request;
use super::decode::decode_ragflow_create;
use super::error::{RagflowError, SearchTransform};
use super::{
    RagflowEnv, endpoints, get_auth_credentials, get_complete_url, search_request, search_response,
    validate_environment,
};
use crate::vector_stores::Headers;
use crate::vector_stores::store::CreateParams;

const BASE: &str = "http://localhost:9380/api/v1/datasets";

fn key(value: &str) -> SecretString {
    SecretString::from(value)
}

fn env(api_key: Option<&str>, api_base: Option<&str>) -> RagflowEnv {
    RagflowEnv {
        api_key: api_key.map(key),
        api_base: api_base.map(str::to_owned),
    }
}

fn params(text: &str) -> CreateParams {
    serde_json::from_str(text).expect("create params")
}

fn plain(headers: &Headers) -> BTreeMap<&str, &str> {
    headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.expose_secret()))
        .collect()
}

fn json_value(text: &str) -> Value {
    serde_json::from_str(text).expect("json")
}

fn body(params: &CreateParams) -> Value {
    create_request(params, BASE).expect("create request").body
}

fn auth_credentials() {
    let sent = get_auth_credentials(Some(&key("explicit-key")), &env(None, None));
    assert_eq!(
        plain(&sent.expect("explicit")),
        BTreeMap::from([("Authorization", "Bearer explicit-key")])
    );
    let from_env = get_auth_credentials(None, &env(Some("env-key"), None));
    assert_eq!(
        plain(&from_env.expect("env")),
        BTreeMap::from([("Authorization", "Bearer env-key")])
    );
    let empty_explicit = get_auth_credentials(Some(&key("")), &env(Some("env-key"), None));
    assert_eq!(
        plain(&empty_explicit.expect("empty explicit counts as set")),
        BTreeMap::from([("Authorization", "Bearer ")])
    );
    let missing = get_auth_credentials(None, &env(None, None)).expect_err("missing key");
    assert!(matches!(missing, RagflowError::MissingAuthKey));
    assert_eq!(
        missing.to_string(),
        "api_key is required (set RAGFLOW_API_KEY env var or pass in litellm_params)"
    );
    let empty_env = get_auth_credentials(None, &env(Some(""), None)).expect_err("empty env");
    assert!(matches!(empty_env, RagflowError::MissingAuthKey));
}

fn validate_headers() {
    let explicit = RagflowEnv::default();
    let sent = validate_environment(Headers::new(), Some(&key("explicit-key")), &explicit);
    assert_eq!(
        plain(&sent.expect("explicit")),
        BTreeMap::from([
            ("Authorization", "Bearer explicit-key"),
            ("Content-Type", "application/json"),
        ])
    );
    let from_env = validate_environment(Headers::new(), None, &env(Some("env-key"), None));
    assert_eq!(
        plain(&from_env.expect("env"))["Authorization"],
        "Bearer env-key"
    );
    let fallback =
        validate_environment(Headers::new(), Some(&key("")), &env(Some("env-key"), None));
    assert_eq!(
        plain(&fallback.expect("empty explicit falls through"))["Authorization"],
        "Bearer env-key"
    );
    // LiteLLM's headers.update replaces a credential the caller already holds
    let stale = validate_environment(
        BTreeMap::from([
            ("Authorization".to_owned(), key("Bearer stale")),
            ("Content-Type".to_owned(), key("text/plain")),
        ]),
        Some(&key("explicit-key")),
        &env(None, None),
    );
    let overwritten = stale.expect("overwritten");
    let replaced = plain(&overwritten);
    assert_eq!(replaced["Authorization"], "Bearer explicit-key");
    assert_eq!(replaced["Content-Type"], "application/json");
    let preserved = validate_environment(
        BTreeMap::from([("X-Trace".to_owned(), key("t"))]),
        Some(&key("explicit-key")),
        &env(None, None),
    );
    assert_eq!(plain(&preserved.expect("preserved"))["X-Trace"], "t");
}

fn validate_missing_key() {
    let no_params = validate_environment(Headers::new(), None, &env(None, None));
    assert!(matches!(
        no_params,
        Err(RagflowError::MissingEnvironmentKey)
    ));
    let error = validate_environment(Headers::new(), None, &env(None, None)).expect_err("missing");
    assert_eq!(
        error.to_string(),
        "RAGFLOW_API_KEY is required (set env var or pass in litellm_params)"
    );
    let empty_explicit = validate_environment(Headers::new(), Some(&key("")), &env(None, None));
    assert!(matches!(
        empty_explicit,
        Err(RagflowError::MissingEnvironmentKey)
    ));
    let empty_env = validate_environment(Headers::new(), None, &env(Some(""), None));
    assert!(matches!(
        empty_env,
        Err(RagflowError::MissingEnvironmentKey)
    ));
}

fn complete_url() {
    assert_eq!(
        get_complete_url(
            Some("https://explicit.example.test///"),
            None,
            &env(None, None)
        ),
        "https://explicit.example.test/api/v1/datasets"
    );
    // the explicit api_base wins over litellm_params and the env
    assert_eq!(
        get_complete_url(
            Some("https://explicit.example.test"),
            Some("https://params.example.test"),
            &env(None, Some("https://env.example.test"))
        ),
        "https://explicit.example.test/api/v1/datasets"
    );
    assert_eq!(
        get_complete_url(
            None,
            Some("https://params.example.test//"),
            &env(None, Some("https://env.example.test"))
        ),
        "https://params.example.test/api/v1/datasets"
    );
    assert_eq!(
        get_complete_url(None, None, &env(None, Some("https://env.example.test/"))),
        "https://env.example.test/api/v1/datasets"
    );
    assert_eq!(
        get_complete_url(None, None, &env(None, None)),
        "http://localhost:9380/api/v1/datasets"
    );
    assert_eq!(
        get_complete_url(Some(""), Some(""), &env(None, Some(""))),
        "http://localhost:9380/api/v1/datasets"
    );
    assert_eq!(
        get_complete_url(Some("/"), None, &env(None, None)),
        "/api/v1/datasets"
    );
}

fn endpoint_table() {
    let routes = endpoints();
    assert_eq!(routes.read, Some(Vec::new()));
    assert_eq!(routes.write, Some(Vec::new()));
    assert!(routes.extra.is_empty());
}

fn create_defaults() {
    let bare = body(&params(r#"{"name":"notes"}"#));
    assert_eq!(bare["name"], "notes");
    assert_eq!(bare["chunk_method"], "naive");
    assert_eq!(bare.as_object().expect("object").len(), 2);
    assert_eq!(body(&params(r#"{"name":"notes","metadata":{}}"#)), bare);
    assert_eq!(
        body(&params(r#"{"name":"notes","file_ids":["file_a"]}"#)),
        bare
    );
}

fn create_missing_name() {
    for text in ["{}", r#"{"name":null}"#, r#"{"name":""}"#] {
        let error = create_request(&params(text), BASE).expect_err("missing name");
        assert!(matches!(error, RagflowError::MissingName));
        assert_eq!(
            error.to_string(),
            "name is required for RAGFlow dataset creation"
        );
    }
    let whitespace = body(&params(r#"{"name":" "}"#));
    assert_eq!(whitespace["name"], " ");
}

fn create_metadata_fields() {
    let sent = body(&params(
        r#"{"name":"notes","metadata":{
            "pipeline_id":"pipe","unknown":"dropped","parse_type":2,
            "parser_config":{"chunk_token_num":512,"delimiter":"\n"},
            "permission":"me","embedding_model":"BAAI/bge","description":"d","avatar":"a"
        }}"#,
    ));
    assert_eq!(
        sent,
        json_value(
            r#"{"name":"notes","avatar":"a","description":"d",
                "embedding_model":"BAAI/bge","permission":"me",
                "parser_config":{"chunk_token_num":512,"delimiter":"\n"},
                "parse_type":2,"pipeline_id":"pipe"}"#
        )
    );
    let chunked = body(&params(
        r#"{"name":"notes","metadata":{"chunk_method":"manual","description":"d"}}"#,
    ));
    assert_eq!(
        chunked,
        json_value(r#"{"name":"notes","description":"d","chunk_method":"manual"}"#)
    );
}

fn create_chunk_method_exclusive() {
    let error = create_request(
        &params(r#"{"name":"notes","metadata":{"chunk_method":"naive","pipeline_id":"pipe"}}"#),
        BASE,
    )
    .expect_err("exclusive");
    assert!(matches!(error, RagflowError::ChunkMethodExclusive));
    assert_eq!(
        error.to_string(),
        "chunk_method and pipeline_id are mutually exclusive. Specify either chunk_method or pipeline_id, not both."
    );
}

fn create_chunk_method_defaults() {
    assert_eq!(
        body(&params(
            r#"{"name":"notes","metadata":{"pipeline_id":"pipe"}}"#
        )),
        json_value(r#"{"name":"notes","pipeline_id":"pipe"}"#)
    );
    assert_eq!(
        body(&params(
            r#"{"name":"notes","metadata":{"chunk_method":null}}"#
        )),
        json_value(r#"{"name":"notes","chunk_method":null}"#)
    );
    assert_eq!(
        body(&params(
            r#"{"name":"notes","metadata":{"pipeline_id":null}}"#
        )),
        json_value(r#"{"name":"notes","pipeline_id":null}"#)
    );
}

fn create_url_is_the_given_base() {
    let request = create_request(
        &params(r#"{"name":"notes"}"#),
        "http://x.test/api/v1/datasets",
    )
    .expect("request");
    assert_eq!(request.url, "http://x.test/api/v1/datasets");
}

fn decode_success() {
    let headers = BTreeMap::from([("x-request-id".to_owned(), "req_1".to_owned())]);
    let body = br#"{"code":0,"data":{"id":"ds_1","name":"notes","create_time":1745836841611}}"#;
    let response = decode_ragflow_create(200, &headers, body).expect("response");
    assert_eq!(
        serde_json::to_value(&response).expect("json"),
        json_value(
            r#"{"id":"ds_1","object":"vector_store","created_at":1745836841,
                "name":"notes","bytes":0,
                "file_counts":{"in_progress":0,"completed":0,"failed":0,"cancelled":0,"total":0},
                "status":"completed","expires_after":null,"expires_at":null,
                "last_active_at":null,"metadata":null}"#
        )
    );
}

fn decode_created_at() {
    let cases: [(&str, i64); 5] = [
        (r#"{"id":"ds_1"}"#, 0),
        (r#"{"id":"ds_1","create_time":0}"#, 0),
        (r#"{"id":"ds_1","create_time":null}"#, 0),
        (r#"{"id":"ds_1","create_time":999}"#, 0),
        (r#"{"id":"ds_1","create_time":-1500}"#, -1),
    ];
    for (data, expected) in cases {
        let body = format!(r#"{{"code":0,"data":{data}}}"#);
        let response =
            decode_ragflow_create(200, &BTreeMap::new(), body.as_bytes()).expect("response");
        assert_eq!(response.created_at, Some(expected));
    }
    let fractional = br#"{"code":0,"data":{"id":"ds_1","create_time":1745836841.611}}"#;
    let fractional_response =
        decode_ragflow_create(200, &BTreeMap::new(), fractional).expect("float time");
    assert_eq!(fractional_response.created_at, Some(1_745_836));
    let negative = br#"{"code":0,"data":{"id":"ds_1","create_time":-1500.7}}"#;
    let negative_response =
        decode_ragflow_create(200, &BTreeMap::new(), negative).expect("negative");
    assert_eq!(negative_response.created_at, Some(-1));
}

fn decode_name_null() {
    let body = br#"{"code":0,"data":{"id":"ds_1","name":null}}"#;
    let response = decode_ragflow_create(200, &BTreeMap::new(), body).expect("null name");
    assert_eq!(response.name, None);
    assert_eq!(
        serde_json::to_value(&response).expect("json"),
        json_value(
            r#"{"id":"ds_1","object":"vector_store","created_at":0,"name":null,
                "bytes":0,
                "file_counts":{"in_progress":0,"completed":0,"failed":0,"cancelled":0,"total":0},
                "status":"completed","expires_after":null,"expires_at":null,
                "last_active_at":null,"metadata":null}"#
        )
    );
}

fn decode_code_value() {
    let success = br#"{"code":0.0,"data":{"id":"ds_1"}}"#;
    decode_ragflow_create(200, &BTreeMap::new(), success).expect("float zero code");
    let sent = BTreeMap::from([("x-request-id".to_owned(), "req_1".to_owned())]);
    let failed = br#"{"code":101,"message":"Dataset name already exists"}"#;
    let error = decode_ragflow_create(400, &sent, failed).expect_err("provider error");
    let RagflowError::ErrorResponse {
        status,
        headers,
        message,
    } = error
    else {
        panic!("not a provider error");
    };
    assert_eq!(status, 400);
    assert_eq!(headers["x-request-id"], "req_1");
    assert_eq!(message, "Dataset name already exists");
    let default_error = decode_ragflow_create(500, &sent, br#"{"code":1}"#).expect_err("default");
    let RagflowError::ErrorResponse {
        status: default_status,
        headers: default_headers,
        message: default_message,
    } = default_error
    else {
        panic!("not a provider error");
    };
    assert_eq!(default_status, 500);
    assert_eq!(default_headers["x-request-id"], "req_1");
    assert_eq!(default_message, "Unknown error");
    let no_code = decode_ragflow_create(502, &sent, b"{}").expect_err("missing code");
    assert!(matches!(
        no_code,
        RagflowError::ErrorResponse { status: 502, .. }
    ));
}

fn decode_unreadable() {
    let sent = BTreeMap::from([("x-request-id".to_owned(), "req_1".to_owned())]);
    let cases: [&[u8]; 5] = [
        b"{",
        b"[]",
        br#"{"code":0,"data":null}"#,
        br#"{"code":0,"data":[]}"#,
        br#"{"code":0,"data":{"id":"ds_1","create_time":"yesterday"}}"#,
    ];
    for body in cases {
        let error = decode_ragflow_create(500, &sent, body).expect_err("unreadable");
        let RagflowError::UnreadableResponse {
            status, headers, ..
        } = error
        else {
            panic!("not an unreadable response");
        };
        assert_eq!(status, 500);
        assert_eq!(headers["x-request-id"], "req_1");
    }
    let huge = br#"{"code":0,"data":{"id":"ds_1","create_time":1e30}}"#;
    let error = decode_ragflow_create(500, &sent, huge).expect_err("out of range");
    assert!(matches!(error, RagflowError::UnreadableResponse { .. }));
}

fn decode_missing_id() {
    let sent = BTreeMap::from([("x-request-id".to_owned(), "req_1".to_owned())]);
    let cases: [&[u8]; 4] = [
        br#"{"code":0,"data":{"name":"notes"}}"#,
        br#"{"code":0,"data":{"id":""}}"#,
        br#"{"code":0,"data":{"id":0}}"#,
        br#"{"code":0}"#,
    ];
    for body in cases {
        let error = decode_ragflow_create(200, &sent, body).expect_err("missing id");
        assert!(matches!(error, RagflowError::MissingDatasetId));
        assert_eq!(error.to_string(), "RAGFlow response missing dataset id");
    }
}

fn search_is_unsupported() {
    let request = search_request().expect_err("search request");
    assert!(matches!(
        request,
        RagflowError::SearchUnsupported {
            transform: SearchTransform::Request
        }
    ));
    assert_eq!(
        request.to_string(),
        "RAGFlow vector stores support dataset management only, not search/retrieval"
    );
    let response = search_response().expect_err("search response");
    assert!(matches!(
        response,
        RagflowError::SearchUnsupported {
            transform: SearchTransform::Response
        }
    ));
    assert_eq!(
        response.to_string(),
        "RAGFlow vector stores support dataset management only, not search/retrieval"
    );
}

#[rstest]
#[case::auth_credentials(auth_credentials)]
#[case::validate_headers(validate_headers)]
#[case::validate_missing_key(validate_missing_key)]
#[case::complete_url(complete_url)]
#[case::endpoint_table(endpoint_table)]
#[case::create_defaults(create_defaults)]
#[case::create_missing_name(create_missing_name)]
#[case::create_metadata_fields(create_metadata_fields)]
#[case::create_chunk_method_exclusive(create_chunk_method_exclusive)]
#[case::create_chunk_method_defaults(create_chunk_method_defaults)]
#[case::create_url_is_the_given_base(create_url_is_the_given_base)]
#[case::decode_success(decode_success)]
#[case::decode_created_at(decode_created_at)]
#[case::decode_name_null(decode_name_null)]
#[case::decode_code_value(decode_code_value)]
#[case::decode_unreadable(decode_unreadable)]
#[case::decode_missing_id(decode_missing_id)]
#[case::search_is_unsupported(search_is_unsupported)]
fn ragflow_vector_store_cases(#[case] scenario: fn()) {
    scenario();
}

#[test]
fn replays_the_ragflow_vector_store_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/ragflow_vector_store");
    let paths: Vec<PathBuf> = fs::read_dir(&corpus)
        .expect("corpus folder")
        .map(|entry| entry.expect("corpus entry").path())
        .collect();
    assert!(!paths.is_empty());
    let headers = BTreeMap::new();
    let failures: Vec<&str> = paths
        .iter()
        .filter(|path| {
            let bytes = fs::read(path).expect("corpus file");
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("name");
            let decoded = decode_ragflow_create(200, &headers, &bytes);
            decoded.is_ok() != name.starts_with("ok-")
        })
        .map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .expect("name")
        })
        .collect();
    assert!(failures.is_empty(), "{failures:?}");
}
