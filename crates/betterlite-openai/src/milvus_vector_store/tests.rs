#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use super::decode::decode_milvus_search;
use super::error::{MilvusError, UnsupportedCreate};
use super::search::{Search, SearchRequest, search_request};
use super::{
    MilvusEnv, MilvusParams, OptionalParams, create_request, create_response, endpoints,
    get_auth_credentials, get_complete_url, map_openai_params, validate_environment,
};
use crate::api::vector_store_records::records::vector::VectorStoreIndexEndpoints;
use crate::vector_stores::Headers;

const BASE: &str = "https://milvus.example.test";

fn key(value: &str) -> SecretString {
    SecretString::from(value)
}

fn params() -> MilvusParams {
    MilvusParams {
        api_key: None,
        milvus_db_name: None,
        milvus_partition_names: None,
        milvus_text_field: None,
        litellm_embedding_model: None,
    }
}

fn env(api_key: Option<&str>, api_base: Option<&str>) -> MilvusEnv {
    MilvusEnv {
        api_key: api_key.map(key),
        api_base: api_base.map(str::to_owned),
    }
}

fn optional(text: &str) -> OptionalParams {
    serde_json::from_str(text).expect("optional params")
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

fn run_search(
    optional_params: &OptionalParams,
    litellm_params: &MilvusParams,
    query_vector: &[f64],
) -> SearchRequest {
    search_request(Search {
        vector_store_id: "docs",
        query_text: "find me",
        query_vector,
        optional_params,
        api_base: BASE,
        litellm_params,
    })
    .expect("search")
}

fn header_pairs(endpoints: &VectorStoreIndexEndpoints) -> (Vec<String>, Vec<String>) {
    let pairs = |list: &Option<Vec<(String, String)>>| {
        list.as_ref()
            .expect("endpoints")
            .iter()
            .map(|(method, path)| format!("{method} {path}"))
            .collect()
    };
    (pairs(&endpoints.read), pairs(&endpoints.write))
}

fn headers() {
    let sent = BTreeMap::from([("X-Trace".to_owned(), key("t"))]);
    let explicit = MilvusParams {
        api_key: Some(key("explicit-key")),
        milvus_db_name: None,
        milvus_partition_names: None,
        milvus_text_field: None,
        litellm_embedding_model: None,
    };
    let both =
        validate_environment(sent, Some(&explicit), &env(Some("env-key"), None)).expect("explicit");
    let shown = plain(&both);
    assert_eq!(shown["Authorization"], "Bearer explicit-key");
    assert_eq!(shown["X-Trace"], "t");
    let from_env =
        validate_environment(Headers::new(), Some(&params()), &env(Some("env-key"), None))
            .expect("env key");
    assert_eq!(plain(&from_env)["Authorization"], "Bearer env-key");
}

fn empty_explicit_key_falls_back() {
    let empty = MilvusParams {
        api_key: Some(key("")),
        milvus_db_name: None,
        milvus_partition_names: None,
        milvus_text_field: None,
        litellm_embedding_model: None,
    };
    let fallback = validate_environment(Headers::new(), Some(&empty), &env(Some("env-key"), None))
        .expect("fallback");
    assert_eq!(plain(&fallback)["Authorization"], "Bearer env-key");
}

fn missing_key() {
    let no_params = validate_environment(Headers::new(), None, &env(Some("env-key"), None));
    assert!(matches!(no_params, Err(MilvusError::MissingApiKey)));
    let no_key = validate_environment(Headers::new(), Some(&params()), &MilvusEnv::default());
    assert!(matches!(no_key, Err(MilvusError::MissingApiKey)));
    let credential = get_auth_credentials(&params()).expect_err("missing credential");
    assert!(matches!(credential, MilvusError::MissingApiKey));
    assert_eq!(
        credential.to_string(),
        "MILVUS_API_KEY is not set. Either set it in the litellm_params or set the MILVUS_API_KEY environment variable."
    );
    let empty = MilvusParams {
        api_key: Some(key("")),
        milvus_db_name: None,
        milvus_partition_names: None,
        milvus_text_field: None,
        litellm_embedding_model: None,
    };
    assert!(matches!(
        get_auth_credentials(&empty),
        Err(MilvusError::MissingApiKey)
    ));
}

fn credentials() {
    let explicit = MilvusParams {
        api_key: Some(key("secret-key")),
        milvus_db_name: None,
        milvus_partition_names: None,
        milvus_text_field: None,
        litellm_embedding_model: None,
    };
    let sent = get_auth_credentials(&explicit).expect("credentials");
    assert_eq!(
        plain(&sent),
        BTreeMap::from([("Authorization", "Bearer secret-key")])
    );
}

fn base_url() {
    let explicit = get_complete_url(Some("https://a.example.test///"), &env(None, None));
    assert_eq!(explicit.expect("explicit"), "https://a.example.test");
    let from_env = get_complete_url(None, &env(None, Some("https://env.example.test//")));
    assert_eq!(from_env.expect("env"), "https://env.example.test");
    let empty_explicit = get_complete_url(Some(""), &env(None, Some("https://env.example.test/")));
    assert_eq!(
        empty_explicit.expect("empty explicit"),
        "https://env.example.test"
    );
    let slashes_only = get_complete_url(Some("/"), &env(None, None));
    assert_eq!(slashes_only.expect("slashes"), "");
}

fn missing_base() {
    let missing = get_complete_url(None, &MilvusEnv::default()).expect_err("missing base");
    assert!(matches!(missing, MilvusError::MissingApiBase));
    assert_eq!(
        missing.to_string(),
        "Milvus API base URL is required. Set MILVUS_API_BASE environment variable or pass api_base in litellm_params."
    );
    let empty_env = get_complete_url(None, &env(None, Some(""))).expect_err("empty env base");
    assert!(matches!(empty_env, MilvusError::MissingApiBase));
}

fn endpoint_table() {
    let (read, write) = header_pairs(&endpoints());
    assert_eq!(
        read,
        [
            "POST /v2/vectordb/entities/search",
            "POST /v2/vectordb/entities/get",
            "POST /v2/vectordb/entities/query",
        ]
    );
    assert_eq!(
        write,
        [
            "POST /v2/vectordb/entities/upsert",
            "POST /v2/vectordb/entities/insert",
        ]
    );
}

fn param_mapping() {
    let non_default = optional(
        r#"{"annsField":"vector","limit":5,"filter":"a > 1","offset":2,
            "groupingField":"group","outputFields":["title"],
            "searchParams":{"nprobe":8},"consistencyLevel":"Strong",
            "unknown":1,"max_num_results":3,"filters":{"eq":true}}"#,
    );
    let optional_params = optional(r#"{"filters":{"base":true},"max_num_results":10}"#);
    let mapped = map_openai_params(&non_default, optional_params, false);
    let expected = optional(
        r#"{"annsField":"vector","limit":5,"filter":"a > 1","offset":2,
            "groupingField":"group","outputFields":["title"],
            "searchParams":{"nprobe":8},"consistencyLevel":"Strong",
            "filters":{"base":true},"max_num_results":10}"#,
    );
    assert_eq!(mapped, expected);
    let unknown = optional(r#"{"unknown":1}"#);
    assert!(map_openai_params(&unknown, OptionalParams::new(), false).is_empty());
}

fn param_mapping_keeps_null_and_zero() {
    let nulled = optional(r#"{"annsField":null}"#);
    assert_eq!(
        map_openai_params(&nulled, OptionalParams::new(), false),
        nulled
    );
    let zero = optional(r#"{"limit":0}"#);
    assert_eq!(map_openai_params(&zero, OptionalParams::new(), false), zero);
}

fn body_order() {
    let bare = run_search(&OptionalParams::new(), &params(), &[0.25_f64, 0.5_f64]);
    assert_eq!(bare.url, format!("{BASE}/v2/vectordb/entities/search"));
    assert_eq!(bare.query_text, "find me");
    assert!(bare.embedding_model.is_none());
    assert_eq!(
        serde_json::to_string(&bare.body).expect("body"),
        r#"{"collectionName":"docs","data":[[0.25,0.5]],"annsField":"book_intro_vector"}"#
    );
    let mapped = optional(r#"{"annsField":"custom_vector","limit":3}"#);
    let custom = run_search(&mapped, &params(), &[0.25_f64, 0.5_f64]);
    assert_eq!(
        serde_json::to_string(&custom.body).expect("body"),
        r#"{"collectionName":"docs","data":[[0.25,0.5]],"annsField":"custom_vector","limit":3}"#
    );
}

fn empty_vector_and_model() {
    let model = MilvusParams {
        api_key: None,
        milvus_db_name: None,
        milvus_partition_names: None,
        milvus_text_field: None,
        litellm_embedding_model: Some("openai/text-embedding-3-small".to_owned()),
    };
    let logged = run_search(&OptionalParams::new(), &model, &[]);
    assert_eq!(
        logged.embedding_model.as_deref(),
        Some("openai/text-embedding-3-small")
    );
    assert_eq!(
        serde_json::to_string(&logged.body).expect("body"),
        r#"{"collectionName":"docs","data":[[]],"annsField":"book_intro_vector"}"#
    );
}

fn scope_overrides() {
    let scoped = MilvusParams {
        api_key: None,
        milvus_db_name: Some("main_db".to_owned()),
        milvus_partition_names: Some(vec!["p_one".to_owned(), "p_two".to_owned()]),
        milvus_text_field: None,
        litellm_embedding_model: None,
    };
    let generated = run_search(&optional(r#"{"limit":3}"#), &scoped, &[0.5_f64]);
    assert_eq!(
        serde_json::to_string(&generated.body).expect("body"),
        r#"{"collectionName":"docs","data":[[0.5]],"annsField":"book_intro_vector","limit":3,"dbName":"main_db","partitionNames":["p_one","p_two"]}"#
    );
    let empty_scope = MilvusParams {
        api_key: None,
        milvus_db_name: Some(String::new()),
        milvus_partition_names: Some(Vec::new()),
        milvus_text_field: None,
        litellm_embedding_model: None,
    };
    let skipped = run_search(
        &optional(r#"{"dbName":"optional_db"}"#),
        &empty_scope,
        &[0.5_f64],
    );
    assert_eq!(
        serde_json::to_string(&skipped.body).expect("body"),
        r#"{"collectionName":"docs","data":[[0.5]],"annsField":"book_intro_vector","dbName":"optional_db"}"#
    );
}

fn scope_replaces_optional() {
    let scoped = MilvusParams {
        api_key: None,
        milvus_db_name: Some("main_db".to_owned()),
        milvus_partition_names: Some(vec!["p_one".to_owned(), "p_two".to_owned()]),
        milvus_text_field: None,
        litellm_embedding_model: None,
    };
    let replaced = run_search(
        &optional(r#"{"dbName":"optional_db","partitionNames":["optional"]}"#),
        &scoped,
        &[0.5_f64],
    );
    assert_eq!(
        serde_json::to_string(&replaced.body).expect("body"),
        r#"{"collectionName":"docs","data":[[0.5]],"annsField":"book_intro_vector","dbName":"main_db","partitionNames":["p_one","p_two"]}"#
    );
}

fn response_projection() {
    let body = br#"{"data":[{"id":"pk1","text":"hello","distance":0.25,"title":"T","rank":2},
        {"id":"pk2","content":"stale","text":"world","distance":0.5},
        {"id":"pk3","text":"zero"}]}"#;
    let response =
        decode_milvus_search(200, &BTreeMap::new(), body, "find me", "text").expect("response");
    let expected = json_value(
        r#"{"object":"vector_store.search_results.page","search_query":"find me","data":[
            {"score":0.25,"content":[{"text":"hello","type":"text"}],
             "file_id":null,"filename":null,"attributes":{"title":"T","rank":2}},
            {"score":0.5,"content":[{"text":"world","type":"text"}],
             "file_id":null,"filename":null,"attributes":{}},
            {"score":0.0,"content":[{"text":"zero","type":"text"}],
             "file_id":null,"filename":null,"attributes":{}}]}"#,
    );
    assert_eq!(serde_json::to_value(&response).expect("json"), expected);
}

fn text_field_lookup() {
    let custom = br#"{"data":[{"id":"1","title":"T","text":"body","distance":0.5}]}"#;
    let titled = decode_milvus_search(200, &BTreeMap::new(), custom, "q", "title")
        .expect("custom text field");
    assert_eq!(
        serde_json::to_value(&titled).expect("json"),
        json_value(
            r#"{"object":"vector_store.search_results.page","search_query":"q","data":[
                {"score":0.5,"content":[{"text":"T","type":"text"}],
                 "file_id":null,"filename":null,"attributes":{"text":"body"}}]}"#
        )
    );
    let unnamed = br#"{"data":[{"id":"1","":"empty-key","text":"kept"}]}"#;
    let blank =
        decode_milvus_search(200, &BTreeMap::new(), unnamed, "q", "").expect("empty text field");
    assert_eq!(
        serde_json::to_value(&blank).expect("json"),
        json_value(
            r#"{"object":"vector_store.search_results.page","search_query":"q","data":[
                {"score":0.0,"content":[{"text":"empty-key","type":"text"}],
                 "file_id":null,"filename":null,"attributes":{"text":"kept"}}]}"#
        )
    );
    let numeric = br#"{"data":[{"id":"1","text":7,"distance":0.25}]}"#;
    let raw = decode_milvus_search(200, &BTreeMap::new(), numeric, "q", "text").expect("raw text");
    assert_eq!(
        serde_json::to_value(&raw).expect("json"),
        json_value(
            r#"{"object":"vector_store.search_results.page","search_query":"q","data":[
                {"score":0.25,"content":[{"text":7,"type":"text"}],
                 "file_id":null,"filename":null,"attributes":{}}]}"#
        )
    );
}

fn response_nulls() {
    let absent = decode_milvus_search(200, &BTreeMap::new(), b"{}", "q", "text").expect("no data");
    assert_eq!(
        serde_json::to_value(&absent).expect("json"),
        json_value(r#"{"object":"vector_store.search_results.page","search_query":"q","data":[]}"#)
    );
    let null_text = br#"{"data":[{"text":null,"distance":null}]}"#;
    let nulled = decode_milvus_search(200, &BTreeMap::new(), null_text, "q", "text")
        .expect("explicit nulls");
    assert_eq!(
        serde_json::to_value(&nulled).expect("json"),
        json_value(
            r#"{"object":"vector_store.search_results.page","search_query":"q","data":[
                {"score":null,"content":[{"text":null,"type":"text"}],
                 "file_id":null,"filename":null,"attributes":{}}]}"#
        )
    );
}

fn response_errors() {
    let sent = BTreeMap::from([("x-request-id".to_owned(), "req_1".to_owned())]);
    let cases: [(u16, &[u8]); 4] = [
        (502, b"{"),
        (500, b"[]"),
        (500, br#"{"data":{}}"#),
        (500, br#"{"data":[{"distance":"close"}]}"#),
    ];
    for (status, body) in cases {
        let error =
            decode_milvus_search(status, &sent, body, "q", "text").expect_err("provider error");
        let MilvusError::Provider {
            status: code,
            headers,
            ..
        } = error
        else {
            panic!("not a provider error");
        };
        assert_eq!(code, status);
        assert_eq!(headers["x-request-id"], "req_1");
    }
}

fn response_score_out_of_range() {
    let bodies: [&[u8]; 2] = [
        br#"{"data":[{"text":"x","distance":1e30}]}"#,
        br#"{"data":[{"text":"x","distance":1.5e-30}]}"#,
    ];
    for body in bodies {
        let error = decode_milvus_search(200, &BTreeMap::new(), body, "q", "text")
            .expect_err("out of range");
        let MilvusError::ScoreOutOfRange(inner) = error else {
            panic!("not a score error");
        };
        let details = inner.details();
        assert_eq!(details.status, Some(502));
        assert_eq!(details.error_type, Some("bad_gateway"));
        assert_eq!(details.param, Some("score"));
        assert_eq!(details.code, Some("decimal_out_of_range"));
    }
}

fn create_is_unsupported() {
    let request = create_request(&OptionalParams::new(), BASE).expect_err("create request");
    assert!(matches!(
        request,
        MilvusError::CreateUnsupported(UnsupportedCreate::Request { .. })
    ));
    let headers = BTreeMap::from([("x-request-id".to_owned(), "req_1".to_owned())]);
    let response = create_response(201, &headers).expect_err("create response");
    assert!(matches!(
        response,
        MilvusError::CreateUnsupported(UnsupportedCreate::Response { .. })
    ));
}

#[rstest]
#[case::headers(headers)]
#[case::empty_explicit_key_falls_back(empty_explicit_key_falls_back)]
#[case::missing_key(missing_key)]
#[case::credentials(credentials)]
#[case::base_url(base_url)]
#[case::missing_base(missing_base)]
#[case::endpoint_table(endpoint_table)]
#[case::param_mapping(param_mapping)]
#[case::param_mapping_keeps_null_and_zero(param_mapping_keeps_null_and_zero)]
#[case::body_order(body_order)]
#[case::empty_vector_and_model(empty_vector_and_model)]
#[case::scope_overrides(scope_overrides)]
#[case::scope_replaces_optional(scope_replaces_optional)]
#[case::response_projection(response_projection)]
#[case::text_field_lookup(text_field_lookup)]
#[case::response_nulls(response_nulls)]
#[case::response_errors(response_errors)]
#[case::response_score_out_of_range(response_score_out_of_range)]
#[case::create_is_unsupported(create_is_unsupported)]
fn milvus_vector_store_cases(#[case] scenario: fn()) {
    scenario();
}

#[test]
fn replays_the_milvus_vector_store_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/milvus_vector_store");
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
            let decoded = decode_milvus_search(200, &headers, &bytes, "query", "text");
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
