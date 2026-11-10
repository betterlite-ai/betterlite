use std::collections::BTreeMap;
use std::sync::Arc;

use http::Method;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::fixtures::{
    ConstantEmbedder, FakeQdrant, Reply, answer, body_of, build, capture, created, default_env,
    details, env_with, exists, field, index_created, json, options,
};
use crate::arguments::json::Json;
use crate::qdrant_semantic_cache::QdrantSemanticCache;
use crate::qdrant_semantic_cache::options::{QdrantEnv, QdrantSemanticOptions};
use crate::qdrant_semantic_cache::ports::QdrantRequest;

fn backend(replies: Vec<Reply>) -> Arc<FakeQdrant> {
    Arc::new(FakeQdrant::new(0.9_f64, replies))
}

fn initialized(
    settings: QdrantSemanticOptions,
    environment: &QdrantEnv,
    replies: Vec<Reply>,
) -> (QdrantSemanticCache, Arc<FakeQdrant>) {
    let transport = backend(replies);
    let cache = build(
        settings,
        environment,
        Arc::clone(&transport),
        Arc::new(ConstantEmbedder::new()),
    )
    .expect("a cache built from the fixture options");
    (cache, transport)
}

async fn initialize_requests(
    settings: QdrantSemanticOptions,
    environment: &QdrantEnv,
    replies: Vec<Reply>,
) -> Vec<QdrantRequest> {
    let (cache, transport) = initialized(settings, environment, replies);
    cache.initialize().await.expect("collection initialization");
    transport.requests()
}

#[test]
fn init_requires_collection_name() {
    let mut settings = options();
    settings.collection_name = None;
    let Err(error) = build(
        settings,
        &default_env(),
        backend(Vec::new()),
        Arc::new(ConstantEmbedder::new()),
    ) else {
        panic!("a cache without a collection name must fail");
    };
    assert_eq!(
        error.to_string(),
        "collection_name must be provided, passed None"
    );
}

#[test]
fn init_requires_similarity_threshold() {
    let mut settings = options();
    settings.similarity_threshold = None;
    let Err(error) = build(
        settings,
        &default_env(),
        backend(Vec::new()),
        Arc::new(ConstantEmbedder::new()),
    ) else {
        panic!("a cache without a threshold must fail");
    };
    assert_eq!(
        error.to_string(),
        "similarity_threshold must be provided, passed None"
    );
}

#[test]
fn env_resolution_prefers_the_explicit_base_and_key() {
    let mut settings = options();
    settings.qdrant_api_key = Some(SecretString::from("explicit-key"));
    let environment = env_with(&[
        ("QDRANT_URL", "http://url.test"),
        ("QDRANT_API_BASE", "http://api-base.test"),
        ("QDRANT_API_KEY", "env-key"),
    ]);
    let cache = build(
        settings,
        &environment,
        backend(Vec::new()),
        Arc::new(ConstantEmbedder::new()),
    )
    .expect("the explicit base and key");
    assert_eq!(cache.api_base, "http://qdrant.test");
    assert_eq!(
        cache
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("explicit-key".to_owned())
    );
}

#[rstest]
#[case::url("QDRANT_URL", "http://url.test")]
#[case::api_base("QDRANT_API_BASE", "http://api-base.test")]
fn env_resolution_reads_the_qdrant_fallbacks(#[case] name: &str, #[case] expected: &str) {
    let mut settings = options();
    settings.qdrant_api_base = None;
    let cache = build(
        settings,
        &env_with(&[(name, expected)]),
        backend(Vec::new()),
        Arc::new(ConstantEmbedder::new()),
    )
    .expect("the fallback base");
    assert_eq!(cache.api_base, expected);
}

#[test]
fn env_resolution_reads_the_qdrant_api_key() {
    let mut settings = options();
    settings.qdrant_api_key = None;
    let cache = build(
        settings,
        &env_with(&[("QDRANT_API_KEY", "env-key")]),
        backend(Vec::new()),
        Arc::new(ConstantEmbedder::new()),
    )
    .expect("the fallback key");
    assert_eq!(
        cache
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("env-key".to_owned())
    );
}

#[test]
fn env_resolution_resolves_os_environ_references() {
    let mut settings = options();
    settings.qdrant_api_base = Some("os.environ/QDRANT_BASE_REF".to_owned());
    settings.qdrant_api_key = Some(SecretString::from("os.environ/QDRANT_KEY_REF"));
    let cache = build(
        settings,
        &env_with(&[
            ("QDRANT_BASE_REF", "http://reference.test"),
            ("QDRANT_KEY_REF", "reference-key"),
        ]),
        backend(Vec::new()),
        Arc::new(ConstantEmbedder::new()),
    )
    .expect("the environment references");
    assert_eq!(cache.api_base, "http://reference.test");
    assert_eq!(
        cache
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("reference-key".to_owned())
    );
}

#[test]
fn env_resolution_treats_an_empty_variable_as_unset() {
    let mut settings = options();
    settings.qdrant_api_base = None;
    let cache = build(
        settings,
        &env_with(&[
            ("QDRANT_URL", ""),
            ("QDRANT_API_BASE", "http://api-base.test"),
        ]),
        backend(Vec::new()),
        Arc::new(ConstantEmbedder::new()),
    )
    .expect("the second fallback base");
    assert_eq!(cache.api_base, "http://api-base.test");
}

#[test]
fn env_missing_url_reports_litellms_failure() {
    let mut settings = options();
    settings.qdrant_api_base = None;
    let Err(error) = build(
        settings,
        &default_env(),
        backend(Vec::new()),
        Arc::new(ConstantEmbedder::new()),
    ) else {
        panic!("a cache without a qdrant url must fail");
    };
    assert_eq!(error.to_string(), "Qdrant url must be provided");
}

#[test]
fn env_missing_variable_names_the_base_field() {
    let mut settings = options();
    settings.qdrant_api_base = Some("os.environ/ABSENT_BASE".to_owned());
    let Err(error) = build(
        settings,
        &default_env(),
        backend(Vec::new()),
        Arc::new(ConstantEmbedder::new()),
    ) else {
        panic!("an absent os.environ reference must fail");
    };
    assert_eq!(
        error.to_string(),
        "environment variable ABSENT_BASE for qdrant_api_base is not set"
    );
}

#[test]
fn env_missing_variable_names_the_key_field() {
    let mut settings = options();
    settings.qdrant_api_key = Some(SecretString::from("os.environ/ABSENT_KEY"));
    let Err(error) = build(
        settings,
        &default_env(),
        backend(Vec::new()),
        Arc::new(ConstantEmbedder::new()),
    ) else {
        panic!("an absent key reference must fail");
    };
    assert_eq!(
        error.to_string(),
        "environment variable ABSENT_KEY for qdrant_api_key is not set"
    );
}

fn settings_env(vector_size: Option<u32>, scalar_quantile: Option<f64>) -> QdrantEnv {
    QdrantEnv {
        values: BTreeMap::new(),
        vector_size,
        scalar_quantile,
    }
}

async fn create_body(settings: QdrantSemanticOptions, environment: &QdrantEnv) -> Json {
    let requests = initialize_requests(
        settings,
        environment,
        vec![exists(false), created(true), details(), index_created()],
    )
    .await;
    body_of(&requests[1])
}

fn quantile_of(body: &Json) -> &Json {
    field(
        field(field(body, "quantization_config"), "scalar"),
        "quantile",
    )
}

fn vector_size_of(body: &Json) -> &Json {
    field(field(body, "vectors"), "size")
}

#[tokio::test]
async fn quantization_binary_is_the_default() {
    let body = create_body(options(), &default_env()).await;
    assert_eq!(
        body,
        json(
            r#"{"vectors":{"size":1536,"distance":"Cosine"},"quantization_config":{"binary":{"always_ram":false}}}"#
        )
    );
}

#[tokio::test]
async fn init_creation_sends_the_full_request_sequence() {
    let requests = initialize_requests(
        options(),
        &default_env(),
        vec![exists(false), created(true), details(), index_created()],
    )
    .await;
    let sent: Vec<(Method, &str)> = requests
        .iter()
        .map(|request| (request.method.clone(), request.path.as_str()))
        .collect();
    assert_eq!(
        sent,
        [
            (Method::GET, "/collections/semantic-cache/exists"),
            (Method::PUT, "/collections/semantic-cache"),
            (Method::GET, "/collections/semantic-cache"),
            (Method::PUT, "/collections/semantic-cache/index"),
        ]
    );
}

#[tokio::test]
async fn quantization_scalar_sends_int8_and_the_quantile() {
    let mut settings = options();
    settings.quantization_config = Some("scalar".to_owned());
    let body = create_body(settings, &default_env()).await;
    assert_eq!(
        field(&body, "quantization_config"),
        &json(r#"{"scalar":{"type":"int8","quantile":0.99,"always_ram":false}}"#)
    );
}

#[tokio::test]
async fn quantization_product_compresses_x16() {
    let mut settings = options();
    settings.quantization_config = Some("product".to_owned());
    let body = create_body(settings, &default_env()).await;
    assert_eq!(
        field(&body, "quantization_config"),
        &json(r#"{"product":{"compression":"x16","always_ram":false}}"#)
    );
}

#[tokio::test]
async fn quantization_binary_and_scalar_and_product_reject_any_other_mode() {
    let mut settings = options();
    settings.quantization_config = Some("ternary".to_owned());
    let (cache, transport) = initialized(settings, &default_env(), vec![exists(false)]);
    let error = cache
        .initialize()
        .await
        .expect_err("an unknown quantization mode must fail");
    assert_eq!(
        error.to_string(),
        "Quantization config must be one of 'scalar', 'binary' or 'product'"
    );
    assert_eq!(transport.requests().len(), 1);
}

#[tokio::test]
async fn vector_size_defaults_to_1536() {
    let body = create_body(options(), &default_env()).await;
    assert_eq!(vector_size_of(&body), &json("1536"));
}

#[tokio::test]
async fn vector_size_argument_overrides_the_config_value() {
    let mut settings = options();
    settings.vector_size = Some(768);
    let body = create_body(settings, &settings_env(Some(1024), None)).await;
    assert_eq!(vector_size_of(&body), &json("768"));
}

#[tokio::test]
async fn vector_size_config_value_overrides_the_default() {
    let body = create_body(options(), &settings_env(Some(1024), None)).await;
    assert_eq!(vector_size_of(&body), &json("1024"));
}

#[tokio::test]
async fn scalar_quantile_defaults_to_0_99() {
    let mut settings = options();
    settings.quantization_config = Some("scalar".to_owned());
    let body = create_body(settings, &default_env()).await;
    assert_eq!(quantile_of(&body), &json("0.99"));
}

#[tokio::test]
async fn scalar_quantile_argument_overrides_the_config_value() {
    let mut settings = options();
    settings.quantization_config = Some("scalar".to_owned());
    settings.scalar_quantile = Some(0.9_f64);
    let body = create_body(settings, &settings_env(None, Some(0.95_f64))).await;
    assert_eq!(quantile_of(&body), &json("0.9"));
}

#[tokio::test]
async fn scalar_quantile_config_value_overrides_the_default() {
    let mut settings = options();
    settings.quantization_config = Some("scalar".to_owned());
    let body = create_body(settings, &settings_env(None, Some(0.95_f64))).await;
    assert_eq!(quantile_of(&body), &json("0.95"));
}

#[tokio::test]
async fn init_existing_collection_skips_the_create_request() {
    let requests = initialize_requests(
        options(),
        &default_env(),
        vec![exists(true), details(), index_created()],
    )
    .await;
    let paths: Vec<&str> = requests
        .iter()
        .map(|request| request.path.as_str())
        .collect();
    assert_eq!(
        paths,
        [
            "/collections/semantic-cache/exists",
            "/collections/semantic-cache",
            "/collections/semantic-cache/index",
        ]
    );
}

#[tokio::test]
async fn init_existing_collection_accepts_an_unknown_quantization_mode() {
    let mut settings = options();
    settings.quantization_config = Some("ternary".to_owned());
    let (cache, _transport) = initialized(
        settings,
        &default_env(),
        vec![exists(true), details(), index_created()],
    );
    cache
        .initialize()
        .await
        .expect("an existing collection is never held to the quantization validation");
}

#[tokio::test]
async fn init_rejects_a_failed_existence_check() {
    let (cache, _transport) = initialized(options(), &default_env(), vec![answer(500, "boom")]);
    let error = cache
        .initialize()
        .await
        .expect_err("a failed existence check must fail initialization");
    assert_eq!(
        error.to_string(),
        "Error from qdrant checking if /collections exist boom"
    );
}

#[tokio::test]
async fn init_rejects_a_creation_that_reports_failure() {
    let (cache, _transport) = initialized(
        options(),
        &default_env(),
        vec![exists(false), created(false)],
    );
    let error = cache
        .initialize()
        .await
        .expect_err("a failed creation must fail initialization");
    assert_eq!(error.to_string(), "Error while creating new collection");
}

#[tokio::test]
async fn init_parses_the_creation_response() {
    let (cache, _transport) = initialized(
        options(),
        &default_env(),
        vec![exists(false), answer(200, "{}")],
    );
    let error = cache
        .initialize()
        .await
        .expect_err("a creation response without result must fail");
    assert!(error.to_string().contains("collection creation"));
    assert!(error.to_string().contains("result"));
}

#[tokio::test]
async fn index_creation_uses_the_keyword_schema() {
    let requests = initialize_requests(
        options(),
        &default_env(),
        vec![exists(true), details(), index_created()],
    )
    .await;
    assert_eq!(
        body_of(&requests[2]),
        json(r#"{"field_name":"litellm_cache_key","field_schema":"keyword"}"#)
    );
}

#[tokio::test]
async fn index_creation_warns_on_a_status_outside_created() {
    let (captured, _guard) = capture();
    let (cache, _transport) = initialized(
        options(),
        &default_env(),
        vec![exists(true), details(), answer(400, "bad request")],
    );
    cache
        .initialize()
        .await
        .expect("an index failure does not stop initialization");
    assert!(
        captured
            .text()
            .contains("could not create cache-key payload index: bad request")
    );
}

#[tokio::test]
async fn index_creation_accepts_a_created_status() {
    let (captured, _guard) = capture();
    let (cache, _transport) = initialized(
        options(),
        &default_env(),
        vec![exists(true), details(), answer(201, "created")],
    );
    cache
        .initialize()
        .await
        .expect("a created index status is not an error");
    assert!(
        !captured
            .text()
            .contains("could not create cache-key payload index")
    );
}

#[tokio::test]
async fn init_parses_the_exists_response() {
    let (cache, _transport) = initialized(options(), &default_env(), vec![answer(200, "{}")]);
    let error = cache
        .initialize()
        .await
        .expect_err("an exists response without result must fail");
    assert!(error.to_string().contains("collection exists"));
    assert!(error.to_string().contains("result"));
}

#[tokio::test]
async fn index_creation_failure_warns_and_does_not_stop_initialization() {
    let (captured, _guard) = capture();
    let (cache, transport) = initialized(
        options(),
        &default_env(),
        vec![exists(true), details(), Reply::Fail],
    );
    cache
        .initialize()
        .await
        .expect("an index failure does not stop initialization");
    assert_eq!(transport.requests().len(), 3);
    assert!(
        captured
            .text()
            .contains("could not create cache-key payload index")
    );
}
