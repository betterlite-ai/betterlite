#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use super::Params;
use super::create::{create_request, create_response, validate_create_vector_store};
use super::decode::decode_mongodb_search;
use super::environment::{
    MongoEnv, auth_credentials, endpoints, get_complete_url, validate_environment,
};
use super::error::{LLM_PROVIDER, MongoDbError, error_for_status};
use super::prepare::{PreparedSearch, prepare_search};
use super::search::{SearchRequest, search_request};
use crate::embedding::{EmbeddingRecord, EmbeddingResponse, EmbeddingUsage, EmbeddingVector};
use crate::vector_stores::Headers;
use crate::vector_stores::store::{Query, SearchParams};

mod corpus;

const BASE: &str = "https://sidecar.example.test";
const VALID: &str = r#"{"litellm_embedding_model":"openai/text-embedding-3-small","mongodb_database":"sample_mflix","mongodb_collection":"embedded_movies"}"#;
const TIMEOUT_CASES: [(&str, u64); 15] = [
    (
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c"}"#,
        30_000,
    ),
    (
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","timeout":null}"#,
        30_000,
    ),
    (
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","timeout":7}"#,
        7_000,
    ),
    (
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","timeout":2.5}"#,
        2_500,
    ),
    (
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","timeout":0.0015}"#,
        1,
    ),
    (
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","timeout":0.0004}"#,
        1,
    ),
    (
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","timeout":{"read":2.5}}"#,
        2_500,
    ),
    (
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","timeout":{"connect":1}}"#,
        30_000,
    ),
    (
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","timeout":{"read":null}}"#,
        30_000,
    ),
    (
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","timeout":{}}"#,
        30_000,
    ),
    (
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","timeout":1e3}"#,
        1_000_000,
    ),
    (
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","timeout":2.50}"#,
        2_500,
    ),
    (
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","timeout":1.10}"#,
        1_100,
    ),
    (
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","timeout":1e-05}"#,
        1,
    ),
    (
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","timeout":{"read":2.50}}"#,
        2_500,
    ),
];

fn key(value: &str) -> SecretString {
    SecretString::from(value)
}

fn params(text: &str) -> Params {
    serde_json::from_str(text).expect("params")
}

fn options(text: &str) -> SearchParams {
    serde_json::from_str(text).expect("search params")
}

fn query(text: &str) -> Query {
    serde_json::from_str(text).expect("query")
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

fn message(error: &MongoDbError) -> String {
    error.to_string()
}

fn prepare(
    litellm_params: &str,
    optional: &str,
    extra_body: Option<&str>,
    text: &str,
) -> Result<PreparedSearch, MongoDbError> {
    let extra = extra_body.map(params);
    prepare_search(
        &params(litellm_params),
        &options(optional),
        extra.as_ref(),
        &query(text),
    )
}

fn embedding(vector: EmbeddingVector) -> EmbeddingResponse {
    EmbeddingResponse {
        object: "list".to_owned(),
        data: vec![EmbeddingRecord {
            object: "embedding".to_owned(),
            embedding: vector,
            index: 0,
        }],
        model: "openai/text-embedding-3-small".to_owned(),
        usage: EmbeddingUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            completion_tokens_details: None,
            prompt_tokens_details: None,
            total_tokens: 0,
        },
    }
}

fn vector(text: &str) -> EmbeddingVector {
    serde_json::from_str(text).expect("embedding vector")
}

fn prepared() -> PreparedSearch {
    prepare(VALID, "{}", None, r#""find me""#).expect("prepared")
}

fn request(vector: EmbeddingVector) -> Result<SearchRequest, MongoDbError> {
    search_request(&prepared(), &embedding(vector), "vs_1", BASE)
}

fn headers() {
    let given = BTreeMap::from([("X-Trace".to_owned(), key("trace"))]);
    let env = MongoEnv {
        api_key: Some(key("env-key")),
    };
    let explicit =
        validate_environment(given, Some(&params(r#"{"api_key":"explicit-key"}"#)), &env)
            .expect("explicit");
    let shown = plain(&explicit);
    assert_eq!(shown["Authorization"], "Bearer explicit-key");
    assert_eq!(shown["Content-Type"], "application/json");
    assert_eq!(shown["X-Trace"], "trace");
    let from_env = validate_environment(Headers::new(), Some(&Params::new()), &env).expect("env");
    assert_eq!(plain(&from_env)["Authorization"], "Bearer env-key");
    let empty = validate_environment(Headers::new(), Some(&params(r#"{"api_key":""}"#)), &env)
        .expect("empty explicit falls back");
    assert_eq!(plain(&empty)["Authorization"], "Bearer env-key");
}

fn header_missing_params() {
    let error =
        validate_environment(Headers::new(), None, &MongoEnv::default()).expect_err("no params");
    assert_eq!(
        message(&error),
        "Configure api_base and api_key for the MongoDB BETA sidecar."
    );
}

fn header_missing_key() {
    let error = validate_environment(Headers::new(), Some(&Params::new()), &MongoEnv::default())
        .expect_err("no key");
    assert_eq!(
        message(&error),
        "MongoDB sidecar api_key is required. Set api_key or MONGODB_SIDECAR_API_KEY."
    );
    let empty = validate_environment(
        Headers::new(),
        Some(&Params::new()),
        &MongoEnv {
            api_key: Some(key("")),
        },
    )
    .expect_err("empty env key");
    assert_eq!(
        message(&empty),
        "MongoDB sidecar api_key is required. Set api_key or MONGODB_SIDECAR_API_KEY."
    );
}

fn header_wrong_types() {
    let wrong = validate_environment(
        Headers::new(),
        Some(&params(r#"{"api_key":5}"#)),
        &MongoEnv::default(),
    )
    .expect_err("non-string key");
    assert_eq!(message(&wrong), "api_key must be a string");
    let unknown = validate_environment(
        Headers::new(),
        Some(&params(r#"{"api_key":"k","mongodb_zzz":1}"#)),
        &MongoEnv::default(),
    )
    .expect_err("unknown parameter");
    assert_eq!(
        message(&unknown),
        "Unrecognised MongoDB vector store parameter(s): mongodb_zzz. Supported: mongodb_collection, mongodb_database, mongodb_embedding_field, mongodb_num_candidates, mongodb_text_field."
    );
}

fn url_accepts() {
    let cases = [
        (
            "https://sidecar.example.test",
            "https://sidecar.example.test",
        ),
        (
            "https://sidecar.example.test///",
            "https://sidecar.example.test",
        ),
        (
            "HTTPS://sidecar.example.test/",
            "HTTPS://sidecar.example.test",
        ),
        ("http://127.0.0.1:8080/", "http://127.0.0.1:8080"),
        ("http://127.0.0.1", "http://127.0.0.1"),
        ("http://[::1]:8080//", "http://[::1]:8080"),
        ("http://[::ffff:127.0.0.1]", "http://[::ffff:127.0.0.1]"),
        ("http://127.0.0.1\t", "http://127.0.0.1\t"),
        (
            "https://sidecar.example.test/prefix",
            "https://sidecar.example.test/prefix",
        ),
        (
            "https://sidecar.example.test?",
            "https://sidecar.example.test?",
        ),
        (
            "https://sidecar.example.test#",
            "https://sidecar.example.test#",
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(
            get_complete_url(Some(input)).expect("url"),
            expected,
            "{input}"
        );
    }
}

fn url_missing() {
    let missing = get_complete_url(None).expect_err("missing");
    assert_eq!(
        message(&missing),
        "MongoDB sidecar api_base is required, for example http://127.0.0.1:8080."
    );
    let empty = get_complete_url(Some("")).expect_err("empty");
    assert_eq!(
        message(&empty),
        "MongoDB sidecar api_base is required, for example http://127.0.0.1:8080."
    );
}

fn url_structural_rejects() {
    let inputs = [
        "https://user@sidecar.example.test",
        "https://user:pass@sidecar.example.test",
        "https://sidecar.example.test?x=1",
        "https://sidecar.example.test#fragment",
        "https://sidecar.example.test:0",
        "ftp://127.0.0.1",
        "not a url",
        "http:127.0.0.1:8080",
        "http:///127.0.0.1",
        "https:example.test",
        "http://evil.com\\@127.0.0.1",
    ];
    for input in inputs {
        let error = get_complete_url(Some(input)).expect_err(input);
        assert_eq!(
            message(&error),
            "MongoDB sidecar api_base must be an HTTP or HTTPS URL without credentials, query, or fragment.",
            "{input}"
        );
    }
}

fn url_http_requires_loopback() {
    let inputs = [
        "http://localhost:8080",
        "http://0.0.0.0",
        "http://127.1",
        "http://2130706433",
        "http://0x7f.0.0.1",
        "http://%31%32%37.0.0.1",
        "http://sidecar.example.test",
    ];
    for input in inputs {
        let error = get_complete_url(Some(input)).expect_err(input);
        assert_eq!(
            message(&error),
            "MongoDB sidecar requires HTTPS. HTTP is supported only for a loopback IP such as 127.0.0.1.",
            "{input}"
        );
    }
}

fn url_parse_rejects() {
    let port =
        get_complete_url(Some("https://sidecar.example.test:99999")).expect_err("invalid port");
    assert_eq!(
        message(&port),
        "MongoDB sidecar api_base must be a valid HTTP or HTTPS URL."
    );
    let brackets = get_complete_url(Some("http://[::1")).expect_err("invalid bracketed host");
    assert_eq!(
        message(&brackets),
        "MongoDB sidecar api_base must be a valid HTTP or HTTPS URL."
    );
}

fn endpoint_table() {
    assert_eq!(
        serde_json::to_value(endpoints()).expect("endpoints"),
        json_value(r#"{"read":[],"write":[]}"#)
    );
    assert_eq!(
        serde_json::to_value(auth_credentials()).expect("credentials"),
        json_value("{}")
    );
}

fn connection_string() {
    let rejected = prepare(
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","mongodb_connection_string":"mongodb://x"}"#,
        "{}",
        None,
        r#""q""#,
    )
    .expect_err("connection string");
    assert_eq!(
        message(&rejected),
        "MongoDB vector stores now use the BETA sidecar. Move mongodb_connection_string to MONGODB_CONNECTION_STRING in the sidecar, remove it from LiteLLM, and configure api_base and api_key."
    );
    let null_rejected = prepare(
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","mongodb_connection_string":null}"#,
        "{}",
        None,
        r#""q""#,
    )
    .expect_err("null connection string is an unknown key");
    assert_eq!(
        message(&null_rejected),
        "Unrecognised MongoDB vector store parameter(s): mongodb_connection_string. Supported: mongodb_collection, mongodb_database, mongodb_embedding_field, mongodb_num_candidates, mongodb_text_field."
    );
}

fn unknown_params() {
    let rejected = prepare(
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","mongodb_zeta":1,"mongodb_alpha":2}"#,
        "{}",
        None,
        r#""q""#,
    )
    .expect_err("unknown parameters");
    assert_eq!(
        message(&rejected),
        "Unrecognised MongoDB vector store parameter(s): mongodb_alpha, mongodb_zeta. Supported: mongodb_collection, mongodb_database, mongodb_embedding_field, mongodb_num_candidates, mongodb_text_field."
    );
    let known = prepare(
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","mongodb_text_field":"body","mongodb_embedding_field":"vector","mongodb_num_candidates":150,"api_key":"k","timeout":2}"#,
        "{}",
        None,
        r#""q""#,
    )
    .expect("known keys");
    assert_eq!(known.text_field, "body");
    assert_eq!(known.embedding_field, "vector");
    assert_eq!(known.num_candidates, 150);
    assert_eq!(known.timeout_ms, 2000);
}

fn extra_body_rules() {
    prepare(VALID, "{}", None, r#""q""#).expect("no extra body");
    prepare(VALID, "{}", Some("{}"), r#""q""#).expect("empty extra body");
    let rejected =
        prepare(VALID, "{}", Some(r#"{"x":null}"#), r#""q""#).expect_err("non-empty extra_body");
    assert_eq!(
        message(&rejected),
        "MongoDB vector store does not support extra_body overrides."
    );
}

fn unsupported_options() {
    let filters = prepare(VALID, r#"{"filters":{}}"#, None, r#""q""#).expect_err("filters");
    assert_eq!(
        message(&filters),
        "MongoDB vector store does not support the filters parameter."
    );
    let ranking =
        prepare(VALID, r#"{"ranking_options":{}}"#, None, r#""q""#).expect_err("ranking options");
    assert_eq!(
        message(&ranking),
        "MongoDB vector store does not support the ranking_options parameter."
    );
    let rewrite = prepare(VALID, r#"{"rewrite_query":false}"#, None, r#""q""#)
        .expect_err("rewrite query false");
    assert_eq!(
        message(&rewrite),
        "MongoDB vector store does not support the rewrite_query parameter."
    );
    let ordered = prepare(
        VALID,
        r#"{"filters":{},"ranking_options":{},"rewrite_query":true}"#,
        None,
        r#""q""#,
    )
    .expect_err("first unsupported wins");
    assert_eq!(
        message(&ordered),
        "MongoDB vector store does not support the filters parameter."
    );
    prepare(
        VALID,
        r#"{"filters":null,"max_num_results":null}"#,
        None,
        r#""q""#,
    )
    .expect("null options are absent");
}

fn typed_parse_failures() {
    let invalid = [
        r#"{"litellm_embedding_model":5,"mongodb_database":"db","mongodb_collection":"c"}"#,
        r#"{"litellm_embedding_model":"m","litellm_embedding_config":[],"mongodb_database":"db","mongodb_collection":"c"}"#,
        r#"{"litellm_embedding_model":"m","litellm_embedding_config":"x","mongodb_database":"db","mongodb_collection":"c"}"#,
        r#"{"litellm_embedding_model":"m","mongodb_database":5,"mongodb_collection":"c"}"#,
        r#"{"litellm_embedding_model":"m","mongodb_database":["db"],"mongodb_collection":"c"}"#,
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":true}"#,
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","mongodb_text_field":5}"#,
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","mongodb_embedding_field":[]}"#,
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","mongodb_num_candidates":"5"}"#,
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","mongodb_num_candidates":5.5}"#,
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","mongodb_num_candidates":true}"#,
    ];
    for input in invalid {
        let error = prepare(input, "{}", None, r#""q""#).expect_err(input);
        assert_eq!(
            message(&error),
            "Invalid MongoDB vector-store configuration. Check the database, collection, fields, and candidate count.",
            "{input}"
        );
    }
}

fn required_database() {
    let error = prepare(
        r#"{"litellm_embedding_model":"m","mongodb_collection":"c"}"#,
        "{}",
        None,
        r#""q""#,
    )
    .expect_err("missing database");
    assert_eq!(
        message(&error),
        "mongodb_database is required in litellm_params for the MongoDB vector store. Example: mongodb_database: sample_mflix"
    );
    let empty = prepare(
        r#"{"litellm_embedding_model":"m","mongodb_database":"","mongodb_collection":"c"}"#,
        "{}",
        None,
        r#""q""#,
    )
    .expect_err("empty database");
    assert_eq!(
        message(&empty),
        "mongodb_database is required in litellm_params for the MongoDB vector store. Example: mongodb_database: sample_mflix"
    );
}

fn required_collection() {
    let error = prepare(
        r#"{"litellm_embedding_model":"m","mongodb_database":"db"}"#,
        "{}",
        None,
        r#""q""#,
    )
    .expect_err("missing collection");
    assert_eq!(
        message(&error),
        "mongodb_collection is required in litellm_params for the MongoDB vector store. Example: mongodb_collection: embedded_movies"
    );
}

fn required_embedding_model() {
    let missing = prepare(
        r#"{"mongodb_database":"db","mongodb_collection":"c"}"#,
        "{}",
        None,
        r#""q""#,
    )
    .expect_err("missing model");
    assert_eq!(
        message(&missing),
        "litellm_embedding_model is required in litellm_params for the MongoDB vector store. It must be the same model that produced the vectors stored in 'c.embedding', or search results will be meaningless. Example: litellm_embedding_model: openai/text-embedding-3-small"
    );
    let empty = prepare(
        r#"{"litellm_embedding_model":"","mongodb_database":"db","mongodb_collection":"c"}"#,
        "{}",
        None,
        r#""q""#,
    )
    .expect_err("empty model");
    assert_eq!(
        message(&empty),
        "litellm_embedding_model is required in litellm_params for the MongoDB vector store. It must be the same model that produced the vectors stored in 'c.embedding', or search results will be meaningless. Example: litellm_embedding_model: openai/text-embedding-3-small"
    );
    let custom = prepare(
        r#"{"mongodb_database":"db","mongodb_collection":"c","mongodb_embedding_field":"vector"}"#,
        "{}",
        None,
        r#""q""#,
    )
    .expect_err("missing model with custom field");
    assert_eq!(
        message(&custom),
        "litellm_embedding_model is required in litellm_params for the MongoDB vector store. It must be the same model that produced the vectors stored in 'c.vector', or search results will be meaningless. Example: litellm_embedding_model: openai/text-embedding-3-small"
    );
}

fn defaults() {
    let defaults = prepared();
    assert_eq!(defaults.query_text, "find me");
    assert_eq!(defaults.embedding_model, "openai/text-embedding-3-small");
    assert_eq!(defaults.embedding_config, Params::new());
    assert_eq!(defaults.database, "sample_mflix");
    assert_eq!(defaults.collection, "embedded_movies");
    assert_eq!(defaults.text_field, "text");
    assert_eq!(defaults.embedding_field, "embedding");
    assert_eq!(defaults.num_candidates, 100);
    assert_eq!(defaults.max_num_results, 10);
    assert_eq!(defaults.timeout_ms, 30_000);
}

fn overrides() {
    let custom = prepare(
        r#"{"litellm_embedding_model":"m","litellm_embedding_config":{"dimensions":2},"mongodb_database":"db","mongodb_collection":"c","mongodb_text_field":"body","mongodb_embedding_field":"vector","mongodb_num_candidates":10000}"#,
        "{}",
        None,
        r#""q""#,
    )
    .expect("custom");
    assert_eq!(custom.embedding_model, "m");
    assert_eq!(custom.database, "db");
    assert_eq!(custom.collection, "c");
    assert_eq!(custom.text_field, "body");
    assert_eq!(custom.embedding_field, "vector");
    assert_eq!(custom.embedding_config, params(r#"{"dimensions":2}"#));
    assert_eq!(custom.num_candidates, 10_000);
}

fn empty_field_defaults() {
    let empty = prepare(
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","mongodb_text_field":"","mongodb_embedding_field":"","litellm_embedding_config":{}}"#,
        "{}",
        None,
        r#""q""#,
    )
    .expect("empty fields");
    assert_eq!(empty.text_field, "text");
    assert_eq!(empty.embedding_field, "embedding");
    assert_eq!(empty.embedding_config, Params::new());
}

fn limit_rules() {
    let zero = prepare(VALID, r#"{"max_num_results":0}"#, None, r#""q""#).expect_err("zero");
    assert_eq!(
        message(&zero),
        "max_num_results must be between 1 and 50, got 0"
    );
    let over = prepare(VALID, r#"{"max_num_results":51}"#, None, r#""q""#).expect_err("51");
    assert_eq!(
        message(&over),
        "max_num_results must be between 1 and 50, got 51"
    );
    let negative =
        prepare(VALID, r#"{"max_num_results":-1}"#, None, r#""q""#).expect_err("negative");
    assert_eq!(
        message(&negative),
        "max_num_results must be between 1 and 50, got -1"
    );
    let low = prepare(VALID, r#"{"max_num_results":1}"#, None, r#""q""#).expect("1");
    assert_eq!(low.max_num_results, 1);
    assert_eq!(low.num_candidates, 100);
    let high = prepare(VALID, r#"{"max_num_results":50}"#, None, r#""q""#).expect("50");
    assert_eq!(high.max_num_results, 50);
    assert_eq!(high.num_candidates, 500);
}

fn candidate_rules() {
    let configured = prepare(
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","mongodb_num_candidates":10000}"#,
        r#"{"max_num_results":50}"#,
        None,
        r#""q""#,
    )
    .expect("10000");
    assert_eq!(configured.num_candidates, 10_000);
    let below = prepare(
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","mongodb_num_candidates":4}"#,
        r#"{"max_num_results":5}"#,
        None,
        r#""q""#,
    )
    .expect_err("below the limit");
    assert_eq!(
        message(&below),
        "mongodb_num_candidates must be between max_num_results (5) and 10000, got 4"
    );
    let over = prepare(
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","mongodb_num_candidates":10001}"#,
        "{}",
        None,
        r#""q""#,
    )
    .expect_err("over the maximum");
    assert_eq!(
        message(&over),
        "mongodb_num_candidates must be between max_num_results (10) and 10000, got 10001"
    );
}

fn timeout_values() {
    for (input, expected) in TIMEOUT_CASES {
        let prepared = prepare(input, "{}", None, r#""q""#).expect(input);
        assert_eq!(prepared.timeout_ms, expected, "{input}");
    }
}

fn timeout_failures() {
    let invalid = [
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","timeout":0}"#,
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","timeout":-1}"#,
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","timeout":"5"}"#,
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","timeout":true}"#,
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","timeout":{"read":"5"}}"#,
    ];
    for input in invalid {
        let error = prepare(input, "{}", None, r#""q""#).expect_err(input);
        assert_eq!(
            message(&error),
            "MongoDB search timeout must be a positive finite number.",
            "{input}"
        );
    }
}

fn query_rules() {
    assert_eq!(prepared().query_text, "find me");
    let joined = prepare(VALID, "{}", None, r#"["one","two","three"]"#).expect("joined");
    assert_eq!(joined.query_text, "one two three");
    let spaced = prepare(VALID, "{}", None, r#"["a","","b"]"#).expect("spaces kept");
    assert_eq!(spaced.query_text, "a  b");
    let padded = prepare(VALID, "{}", None, r#""  a  ""#).expect("padding kept");
    assert_eq!(padded.query_text, "  a  ");
    let blank = prepare(VALID, "{}", None, r#""   ""#).expect_err("blank");
    assert_eq!(message(&blank), "query must not be empty");
    let empty_list = prepare(VALID, "{}", None, "[]").expect_err("empty list");
    assert_eq!(message(&empty_list), "query must not be empty");
}

fn query_length() {
    let too_long = "x".repeat(32_001);
    let error = prepare(VALID, "{}", None, &format!("{too_long:?}")).expect_err("32001");
    assert_eq!(
        message(&error),
        "query must be at most 32000 characters, got 32001"
    );
    let longest = "x".repeat(32_000);
    let accepted = prepare(VALID, "{}", None, &format!("{longest:?}")).expect("32000");
    assert_eq!(accepted.query_text.chars().count(), 32_000);
    let unicode = "\u{e9}".repeat(32_001);
    let unicode_error =
        prepare(VALID, "{}", None, &format!("{unicode:?}")).expect_err("unicode too long");
    assert_eq!(
        message(&unicode_error),
        "query must be at most 32000 characters, got 32001"
    );
}

fn request_body_defaults() {
    let built = request(vector("[0.25,1]")).expect("request");
    assert_eq!(
        built.url,
        "https://sidecar.example.test/v1/vector_stores/vs_1/search"
    );
    assert_eq!(
        serde_json::to_value(&built.body).expect("body"),
        json_value(
            r#"{"query":"find me","query_vector":[0.25,1],"mongodb_database":"sample_mflix",
                "mongodb_collection":"embedded_movies","mongodb_embedding_field":"embedding",
                "mongodb_text_field":"text","mongodb_num_candidates":100,
                "max_num_results":10,"timeout_ms":30000}"#
        )
    );
}

fn request_body_custom() {
    let custom = prepare(
        r#"{"litellm_embedding_model":"m","mongodb_database":"db","mongodb_collection":"c","mongodb_text_field":"body","mongodb_embedding_field":"vector","mongodb_num_candidates":10000,"timeout":2.5}"#,
        r#"{"max_num_results":50}"#,
        None,
        r#"["one","two"]"#,
    )
    .expect("custom prepared");
    let built = search_request(
        &custom,
        &embedding(vector("[0.5]")),
        "vs/a b",
        "http://127.0.0.1:8080",
    )
    .expect("custom request");
    assert_eq!(
        built.url,
        "http://127.0.0.1:8080/v1/vector_stores/vs%2Fa%20b/search"
    );
    let body = serde_json::to_value(&built.body).expect("body");
    assert_eq!(body["query"], json_value(r#""one two""#));
    assert_eq!(body["query_vector"], json_value("[0.5]"));
    assert_eq!(body["mongodb_database"], json_value(r#""db""#));
    assert_eq!(body["mongodb_collection"], json_value(r#""c""#));
    assert_eq!(body["mongodb_embedding_field"], json_value(r#""vector""#));
    assert_eq!(body["mongodb_text_field"], json_value(r#""body""#));
    assert_eq!(body["mongodb_num_candidates"], json_value("10000"));
    assert_eq!(body["max_num_results"], json_value("50"));
    assert_eq!(body["timeout_ms"], json_value("2500"));
}

fn request_encoding() {
    let encoded =
        search_request(&prepared(), &embedding(vector("[1]")), "a/b c?", BASE).expect("encoded");
    assert_eq!(
        encoded.url,
        "https://sidecar.example.test/v1/vector_stores/a%2Fb%20c%3F/search"
    );
    let dotted =
        search_request(&prepared(), &embedding(vector("[1]")), "..", BASE).expect("dot segment");
    assert_eq!(
        dotted.url,
        "https://sidecar.example.test/v1/vector_stores/../search"
    );
    let empty = search_request(&prepared(), &embedding(vector("[1]")), "", BASE).expect("empty id");
    assert_eq!(
        empty.url,
        "https://sidecar.example.test/v1/vector_stores//search"
    );
    let unreserved =
        search_request(&prepared(), &embedding(vector("[1]")), "A-z_0.9~", BASE).expect("kept");
    assert_eq!(
        unreserved.url,
        "https://sidecar.example.test/v1/vector_stores/A-z_0.9~/search"
    );
}

fn request_rejects_embedding() {
    let mut empty_data = embedding(vector("[]"));
    empty_data.data.clear();
    let no_data = search_request(&prepared(), &empty_data, "vs_1", BASE).expect_err("no data");
    assert_eq!(
        message(&no_data),
        "The embedding model returned no embedding for the search query. Check litellm_embedding_model."
    );
    let empty_vector = request(vector("[]")).expect_err("empty vector");
    assert_eq!(
        message(&empty_vector),
        "The embedding model must return a non-empty, finite query vector."
    );
    let base64 = request(vector(r#""AAAA""#)).expect_err("base64 vector");
    assert_eq!(
        message(&base64),
        "The embedding model must return a non-empty, finite query vector."
    );
}

fn request_uses_first_record() {
    let mut response = embedding(vector("[0.25]"));
    let second = EmbeddingRecord {
        object: "embedding".to_owned(),
        embedding: vector("[9]"),
        index: 1,
    };
    response.data.push(second);
    let built = search_request(&prepared(), &response, "vs_1", BASE).expect("first record");
    let body = serde_json::to_value(&built.body).expect("body");
    assert_eq!(body["query_vector"], json_value("[0.25]"));
}

fn decode_projection() {
    let body = br#"{"object":"vector_store.search_results.page","search_query":"find me","data":[
        {"score":0.25,"content":[{"type":"text","text":"hello"},{"type":"text","text":"world"}],
         "file_id":"file_a","filename":"a.txt","attributes":{"ignored":true}},
        {"score":7,"content":[],"file_id":null,"filename":null},
        {"score":null,"content":[],"file_id":"file_b","filename":null}
    ],"extra":"ignored"}"#;
    let decoded = decode_mongodb_search(body).expect("decoded");
    assert_eq!(
        serde_json::to_value(&decoded).expect("json"),
        json_value(
            r#"{"object":"vector_store.search_results.page","search_query":"find me","data":[
                {"score":0.25,"content":[{"type":"text","text":"hello"},{"type":"text","text":"world"}],
                 "file_id":"file_a","filename":"a.txt"},
                {"score":7,"content":[],"file_id":null,"filename":null},
                {"score":null,"content":[],"file_id":"file_b","filename":null}
            ]}"#
        )
    );
}

fn decode_empty_rows() {
    let decoded = decode_mongodb_search(
        br#"{"object":"vector_store.search_results.page","search_query":"","data":[]}"#,
    )
    .expect("empty data");
    assert_eq!(
        serde_json::to_value(&decoded).expect("json"),
        json_value(r#"{"object":"vector_store.search_results.page","search_query":"","data":[]}"#)
    );
}

fn decode_failures() {
    let invalid: [&[u8]; 27] = [
        b"",
        b"not json",
        b"[]",
        b"{}",
        br#"{"object":"other","search_query":"q","data":[]}"#,
        br#"{"object":"vector_store.search_results.page","data":[]}"#,
        br#"{"object":"vector_store.search_results.page","search_query":5,"data":[]}"#,
        br#"{"object":"vector_store.search_results.page","search_query":"q"}"#,
        br#"{"object":"vector_store.search_results.page","search_query":"q","data":null}"#,
        br#"{"object":"vector_store.search_results.page","search_query":"q","data":{}}"#,
        br#"{"object":"vector_store.search_results.page","search_query":"q","data":[{}]}"#,
        br#"{"object":"vector_store.search_results.page","search_query":"q","data":[{"content":[],"file_id":null,"filename":null}]}"#,
        br#"{"object":"vector_store.search_results.page","search_query":"q","data":[{"score":1,"file_id":null,"filename":null}]}"#,
        br#"{"object":"vector_store.search_results.page","search_query":"q","data":[{"score":1,"content":[],"filename":null}]}"#,
        br#"{"object":"vector_store.search_results.page","search_query":"q","data":[{"score":1,"content":[],"file_id":null}]}"#,
        br#"{"object":"vector_store.search_results.page","search_query":"q","data":[{"score":"x","content":[],"file_id":null,"filename":null}]}"#,
        br#"{"object":"vector_store.search_results.page","search_query":"q","data":[{"score":true,"content":[],"file_id":null,"filename":null}]}"#,
        br#"{"object":"vector_store.search_results.page","search_query":"q","data":[{"score":1,"content":{},"file_id":null,"filename":null}]}"#,
        br#"{"object":"vector_store.search_results.page","search_query":"q","data":[{"score":1,"content":[{"type":"image","text":"x"}],"file_id":null,"filename":null}]}"#,
        br#"{"object":"vector_store.search_results.page","search_query":"q","data":[{"score":1,"content":[{"type":"text"}],"file_id":null,"filename":null}]}"#,
        br#"{"object":"vector_store.search_results.page","search_query":"q","data":[{"score":1,"content":[{"type":"text","text":5}],"file_id":null,"filename":null}]}"#,
        br#"{"object":"vector_store.search_results.page","search_query":"q","data":[{"score":1,"content":[],"file_id":5,"filename":null}]}"#,
        br#"{"object":"vector_store.search_results.page","search_query":"q","data":[{"score":1e300,"content":[],"file_id":null,"filename":null}]}"#,
        br#"{"object":"vector_store.search_results.page","search_query":"q","data":[{"score":1e400,"content":[],"file_id":null,"filename":null}]}"#,
        br#"{"object":"vector_store.search_results.page","search_query":"q","data":[{"score":NaN,"content":[],"file_id":null,"filename":null}]}"#,
        br#"{"object":{"vector_store.search_results.page":null},"search_query":"q","data":[]}"#,
        br#"{"object":"vector_store.search_results.page","search_query":"q","data":[{"score":1.5,"content":[{"type":{"text":null},"text":"x"}],"file_id":null,"filename":null}]}"#,
    ];
    for body in invalid {
        let error = decode_mongodb_search(body).expect_err("invalid response");
        assert_eq!(
            message(&error),
            "MongoDB sidecar returned an invalid search response. Check the sidecar version and deployment."
        );
    }
}

fn status_mapping() {
    let bad_request = error_for_status("sidecar text".to_owned(), 400);
    assert!(matches!(bad_request, MongoDbError::BadRequest { .. }));
    assert_eq!(message(&bad_request), "sidecar text");
    let authentication = error_for_status("ignored".to_owned(), 401);
    assert!(matches!(authentication, MongoDbError::Authentication));
    assert_eq!(
        message(&authentication),
        "MongoDB sidecar rejected api_key."
    );
    let timeout = error_for_status("sidecar text".to_owned(), 408);
    assert!(matches!(timeout, MongoDbError::Timeout { .. }));
    assert_eq!(message(&timeout), "sidecar text");
    let unavailable = error_for_status("ignored".to_owned(), 500);
    assert!(matches!(
        unavailable,
        MongoDbError::ServiceUnavailable { .. }
    ));
    assert_eq!(
        message(&unavailable),
        "MongoDB sidecar is unavailable. Check its address, health, and logs."
    );
    let other = error_for_status("ignored".to_owned(), 418);
    assert_eq!(
        message(&other),
        "MongoDB sidecar is unavailable. Check its address, health, and logs."
    );
}

fn create_is_search_only() {
    let expected = "MongoDB vector store is search-only. Create the collection and its MongoDB Vector Search index in MongoDB directly, then register it here by index name.";
    let validate = validate_create_vector_store().expect_err("validate create");
    assert_eq!(message(&validate), expected);
    let build = create_request().expect_err("create request");
    assert_eq!(message(&build), expected);
    let response = create_response().expect_err("create response");
    assert_eq!(message(&response), expected);
}

fn provider_name() {
    assert_eq!(LLM_PROVIDER, "mongodb");
}

#[rstest]
#[case::headers(headers)]
#[case::header_missing_params(header_missing_params)]
#[case::header_missing_key(header_missing_key)]
#[case::header_wrong_types(header_wrong_types)]
#[case::url_accepts(url_accepts)]
#[case::url_missing(url_missing)]
#[case::url_structural_rejects(url_structural_rejects)]
#[case::url_http_requires_loopback(url_http_requires_loopback)]
#[case::url_parse_rejects(url_parse_rejects)]
#[case::endpoint_table(endpoint_table)]
#[case::connection_string(connection_string)]
#[case::unknown_params(unknown_params)]
#[case::extra_body_rules(extra_body_rules)]
#[case::unsupported_options(unsupported_options)]
#[case::typed_parse_failures(typed_parse_failures)]
#[case::required_database(required_database)]
#[case::required_collection(required_collection)]
#[case::required_embedding_model(required_embedding_model)]
#[case::defaults(defaults)]
#[case::overrides(overrides)]
#[case::empty_field_defaults(empty_field_defaults)]
#[case::limit_rules(limit_rules)]
#[case::candidate_rules(candidate_rules)]
#[case::timeout_values(timeout_values)]
#[case::timeout_failures(timeout_failures)]
#[case::query_rules(query_rules)]
#[case::query_length(query_length)]
#[case::request_body_defaults(request_body_defaults)]
#[case::request_body_custom(request_body_custom)]
#[case::request_encoding(request_encoding)]
#[case::request_rejects_embedding(request_rejects_embedding)]
#[case::request_uses_first_record(request_uses_first_record)]
#[case::decode_projection(decode_projection)]
#[case::decode_empty_rows(decode_empty_rows)]
#[case::decode_failures(decode_failures)]
#[case::status_mapping(status_mapping)]
#[case::create_is_search_only(create_is_search_only)]
#[case::provider_name(provider_name)]
fn mongodb_vector_store_cases(#[case] scenario: fn()) {
    scenario();
}
