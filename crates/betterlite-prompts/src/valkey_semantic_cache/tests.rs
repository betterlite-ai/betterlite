#![cfg(test)]

mod fixtures;

use std::sync::Arc;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use self::fixtures::{
    FakeEmbedder, FakeIndex, INDEX_NAME, PROMPT, VECTOR, alternative_error, build, cache_with,
    cache_with_injected_index, command_args, empty_env, exists_error, hit, info_reply,
    kwargs_with_message, kwargs_without_prompt, options, packed_vector, server_error, text_args,
};
use crate::arguments::json::{Json, Object};
use crate::redis_semantic_cache::SIMILARITY_FIELD;
use crate::valkey_semantic_cache::ValkeySemanticCache;
use crate::valkey_semantic_cache::live::{create_command, search_command, store_command};
use crate::valkey_semantic_cache::options::ValkeyEnv;
use crate::valkey_semantic_cache::ports::{IndexSchema, ValkeyIndex};
use crate::valkey_semantic_cache::schema::{
    assert_dim_matches, doc_mapping, extract_index_dim, knn_query, pack_vector, scope_tag,
};

fn url_of(cache: &ValkeySemanticCache) -> String {
    cache
        .url
        .as_ref()
        .expect("a resolved valkey url")
        .expose_secret()
        .to_owned()
}

fn similarity_of(metadata: &Object) -> f64 {
    match metadata.get(SIMILARITY_FIELD) {
        Some(Json::Number(number)) => number.as_f64().expect("a finite similarity"),
        _ => panic!("the semantic similarity is missing"),
    }
}

fn assert_similarity(metadata: &Object, expected: f64) {
    assert_eq!(similarity_of(metadata).to_bits(), expected.to_bits());
}

#[test]
fn options_missing_threshold_fails_with_litellms_message() {
    let mut settings = options();
    settings.similarity_threshold = None;
    let Err(error) = build(settings, empty_env()) else {
        panic!("a cache without a threshold must fail");
    };
    assert_eq!(
        error.to_string(),
        "similarity_threshold must be provided, passed None"
    );
}

#[test]
fn options_startup_nodes_fail_with_litellms_message() {
    let mut settings = options();
    settings.startup_nodes = vec!["valkey-0001:6379".to_owned()];
    let Err(error) = build(settings, empty_env()) else {
        panic!("a cluster-mode endpoint must fail");
    };
    assert_eq!(
        error.to_string(),
        "valkey-semantic does not support cluster-mode-enabled (multi-shard) endpoints. The \
         async cluster client cannot route the FT.* search commands reliably. Point it at a \
         cluster-mode-disabled endpoint instead (a primary with replicas is fine; only \
         horizontal sharding is unsupported), or pass a single redis_url. On AWS, vector search \
         needs ElastiCache for Valkey 8.2+ on a node-based cluster."
    );
}

#[test]
fn options_missing_threshold_is_reported_before_startup_nodes() {
    let mut settings = options();
    settings.similarity_threshold = None;
    settings.startup_nodes = vec!["valkey-0001:6379".to_owned()];
    let Err(error) = build(settings, empty_env()) else {
        panic!("the threshold check comes first");
    };
    assert_eq!(
        error.to_string(),
        "similarity_threshold must be provided, passed None"
    );
}

#[rstest]
#[case::default_name(None, INDEX_NAME)]
#[case::configured_name(Some("custom-index"), "custom-index")]
fn options_index_name_and_key_prefix(#[case] index_name: Option<&str>, #[case] expected: &str) {
    let mut settings = options();
    settings.index_name = index_name.map(str::to_owned);
    let cache = build(settings, empty_env()).expect("the index settings");
    assert_eq!(cache.index_name, expected);
    assert_eq!(cache.key_prefix, format!("{expected}:"));
}

#[rstest]
#[case::valkey_wins(
    ValkeyEnv {
        valkey_host: Some("valkey.internal".to_owned()),
        valkey_port: Some("6380".to_owned()),
        valkey_password: Some(SecretString::from("envpw")),
        redis_host: Some("redis.internal".to_owned()),
        redis_port: Some("6381".to_owned()),
        redis_password: Some(SecretString::from("redispw")),
    },
    "redis://:envpw@valkey.internal:6380"
)]
#[case::redis_falls_back(
    ValkeyEnv {
        valkey_host: None,
        valkey_port: None,
        valkey_password: None,
        redis_host: Some("redis.internal".to_owned()),
        redis_port: Some("6381".to_owned()),
        redis_password: Some(SecretString::from("redispw")),
    },
    "redis://:redispw@redis.internal:6381"
)]
#[case::an_empty_valkey_value_falls_back(
    ValkeyEnv {
        valkey_host: Some(String::new()),
        valkey_port: Some(String::new()),
        valkey_password: Some(SecretString::from("")),
        redis_host: Some("redis.internal".to_owned()),
        redis_port: Some("6381".to_owned()),
        redis_password: Some(SecretString::from("redispw")),
    },
    "redis://:redispw@redis.internal:6381"
)]
fn options_env_values_fill_the_missing_host_port_and_password(
    #[case] env: ValkeyEnv,
    #[case] expected: &str,
) {
    let mut settings = options();
    settings.host = None;
    settings.port = None;
    settings.password = None;
    let cache = build(settings, env).expect("the environment settings");
    assert_eq!(url_of(&cache), expected);
}

#[rstest]
#[case::host(None, Some("6379"))]
#[case::port(Some("localhost"), None)]
fn options_missing_host_or_port_fails_with_litellms_message(
    #[case] host: Option<&str>,
    #[case] port: Option<&str>,
) {
    let mut settings = options();
    settings.host = host.map(str::to_owned);
    settings.port = port.map(str::to_owned);
    let Err(error) = build(settings, empty_env()) else {
        panic!("a missing host or port must fail");
    };
    assert_eq!(
        error.to_string(),
        "Missing required Valkey configuration. Provide host and port (or VALKEY_HOST/VALKEY_PORT), \
         or pass redis_url."
    );
}

#[rstest]
#[case::an_empty_explicit_host_loses_to_the_environment(String::new(), "envhost")]
#[case::a_set_explicit_host_wins("explicit", "explicit")]
fn options_explicit_values_win_and_empty_strings_count_as_missing(
    #[case] host: String,
    #[case] expected: &str,
) {
    let mut settings = options();
    settings.host = Some(host);
    let env = ValkeyEnv {
        valkey_host: Some("envhost".to_owned()),
        ..empty_env()
    };
    let cache = build(settings, env).expect("the host choice");
    assert_eq!(url_of(&cache), format!("redis://:pw@{expected}:6379"));
}

#[test]
fn options_redis_url_wins_over_host_port_and_password() {
    let mut settings = options();
    settings.redis_url = Some(SecretString::from("redis://:urlpw@elsewhere:6390"));
    settings.host = Some("ignored".to_owned());
    let cache = build(settings, empty_env()).expect("the url setting");
    assert_eq!(url_of(&cache), "redis://:urlpw@elsewhere:6390");
}

#[test]
fn options_ssl_selects_the_rediss_scheme() {
    let mut settings = options();
    settings.ssl = true;
    let cache = build(settings, empty_env()).expect("the ssl setting");
    assert_eq!(url_of(&cache), "rediss://:pw@localhost:6379");
}

#[rstest]
#[case::punctuation("p@ss word/", "redis://:p%40ss%20word%2F@localhost:6379")]
#[case::python_safe_characters("a~b_c.d-e", "redis://:a~b_c.d-e@localhost:6379")]
fn options_password_is_percent_encoded_like_python_quote(
    #[case] password: &str,
    #[case] expected: &str,
) {
    let mut settings = options();
    settings.password = Some(SecretString::from(password));
    let cache = build(settings, empty_env()).expect("the password encoding");
    assert_eq!(url_of(&cache), expected);
}

#[test]
fn options_a_missing_password_builds_a_credential_free_url() {
    let mut settings = options();
    settings.password = None;
    let cache = build(settings, empty_env()).expect("the credential-free url");
    assert_eq!(url_of(&cache), "redis://localhost:6379");
}

#[test]
fn options_an_injected_index_needs_no_url() {
    let backend: Arc<dyn ValkeyIndex> = Arc::new(FakeIndex::default());
    let mut settings = options();
    settings.host = None;
    settings.port = None;
    settings.password = None;
    settings.redis_url = None;
    settings.index = Some(backend);
    let cache = build(settings, empty_env()).expect("the injected index");
    assert!(cache.url.is_none());
}

#[test]
fn options_nothing_is_opened_until_first_use() {
    let index = Arc::new(FakeIndex::default());
    let (cache, connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    assert!(connector.connects().is_empty());
    assert!(cache.url.is_some());
}

#[test]
fn options_embedding_settings_are_kept_public() {
    let cache = build(options(), empty_env()).expect("the embedding defaults");
    assert_eq!(cache.embedding_model, "text-embedding-ada-002");
    assert!(cache.embedding_max_input_tokens.is_none());
    assert_eq!(cache.embedding_timeout.to_bits(), 5.0_f64.to_bits());
}

#[rstest]
#[case::ascii(
    "user-1",
    "c6c289e49e9c05b2145860387b73bcb18df43fb09a1e4a4a9713c76c88bb541b"
)]
#[case::unicode(
    "h\u{e9}llo \u{2603}",
    "c3a3e84b30327c8e3f04a788d8b68da3fd94c22353c5a30e3fe6ca7ebde3ab3f"
)]
fn tag_scope_tag_is_the_lowercase_hex_sha256_of_the_key(#[case] key: &str, #[case] expected: &str) {
    assert_eq!(scope_tag(key), expected);
}

#[tokio::test]
async fn tag_documents_are_prefixed_with_the_scope_tag_and_a_new_uuid() {
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let kwargs = kwargs_with_message(PROMPT);
    cache
        .async_set_cache("user-1", &Json::Text("first".to_owned()), &kwargs)
        .await;
    cache
        .async_set_cache("user-1", &Json::Text("second".to_owned()), &kwargs)
        .await;
    let stores = index.stores();
    assert_eq!(stores.len(), 2);
    let scope = scope_tag("user-1");
    assert_eq!(
        stores[0].0,
        format!("{INDEX_NAME}:{scope}:00000000-0000-4000-8000-000000000000")
    );
    assert_eq!(
        stores[1].0,
        format!("{INDEX_NAME}:{scope}:00000000-0000-4000-8000-000000000001")
    );
}

#[tokio::test]
async fn tag_doc_mapping_carries_the_litellm_fields() {
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    cache
        .async_set_cache(
            "user-1",
            &Json::Text("cached answer".to_owned()),
            &kwargs_with_message(PROMPT),
        )
        .await;
    let stores = index.stores();
    let fields = &stores[0].1;
    assert_eq!(fields.litellm_cache_key, scope_tag("user-1"));
    assert_eq!(fields.prompt, PROMPT);
    assert_eq!(fields.response, "\"cached answer\"");
    assert_eq!(fields.embedding, packed_vector());
}

#[test]
fn tag_pack_vector_is_little_endian_f32() {
    assert_eq!(
        pack_vector(&[1.0_f32, -2.5_f32]),
        vec![0_u8, 0, 128, 63, 0, 0, 32, 192]
    );
}

#[test]
fn schema_index_schema_matches_litellm() {
    let schema = IndexSchema {
        prefix: format!("{INDEX_NAME}:"),
        dimension: 3,
    };
    assert_eq!(
        text_args(&create_command(INDEX_NAME, &schema)),
        vec![
            "FT.CREATE",
            INDEX_NAME,
            "ON",
            "HASH",
            "PREFIX",
            "1",
            "litellm_semantic_cache_index:",
            "SCHEMA",
            "litellm_cache_key",
            "TAG",
            "SEPARATOR",
            ",",
            "embedding",
            "VECTOR",
            "HNSW",
            "6",
            "TYPE",
            "FLOAT32",
            "DIM",
            "3",
            "DISTANCE_METRIC",
            "COSINE",
        ]
    );
}

#[test]
fn schema_knn_query_matches_litellm() {
    let query = knn_query("user-1");
    assert_eq!(
        query.text,
        format!(
            "(@litellm_cache_key:{{{}}})=>[KNN 1 @embedding $vec AS vector_distance]",
            scope_tag("user-1")
        )
    );
    assert_eq!(query.return_fields, vec!["response", "vector_distance"]);
    assert_eq!(query.dialect, 2);
}

#[test]
fn schema_search_command_sends_the_knn_query() {
    let query = knn_query("user-1");
    let args = command_args(&search_command(INDEX_NAME, &query, &packed_vector()));
    let text: Vec<String> = args
        .iter()
        .take(15)
        .map(|bytes| String::from_utf8(bytes.clone()).expect("a text argument"))
        .collect();
    assert_eq!(
        text,
        vec![
            "FT.SEARCH",
            INDEX_NAME,
            query.text.as_str(),
            "RETURN",
            "2",
            "response",
            "vector_distance",
            "DIALECT",
            "2",
            "LIMIT",
            "0",
            "10",
            "PARAMS",
            "2",
            "vec",
        ]
    );
    assert_eq!(args.last(), Some(&packed_vector()));
}

#[test]
fn schema_store_command_writes_the_litellm_fields() {
    let fields = doc_mapping("user-1", PROMPT, "{}", &VECTOR);
    let args = command_args(&store_command("doc-key", &fields));
    let text: Vec<String> = args
        .iter()
        .take(9)
        .map(|bytes| String::from_utf8(bytes.clone()).expect("a text argument"))
        .collect();
    assert_eq!(
        text,
        vec![
            "HSET",
            "doc-key",
            "litellm_cache_key",
            scope_tag("user-1").as_str(),
            "prompt",
            PROMPT,
            "response",
            "{}",
            "embedding",
        ]
    );
    assert_eq!(args.last(), Some(&packed_vector()));
}

#[test]
fn schema_extract_index_dim_reads_a_nested_dimensions_marker() {
    assert_eq!(extract_index_dim(&info_reply(768)), Some(768));
}

#[rstest]
#[case::number(Json::Number(serde_json::Number::from(512_u64)))]
#[case::text(Json::Text("512".to_owned()))]
fn schema_extract_index_dim_reads_the_marker_value(#[case] dimension: Json) {
    let attribute = Json::List(vec![
        Json::Text("identifier".to_owned()),
        Json::Text("embedding".to_owned()),
        Json::List(vec![Json::Text("dimensions".to_owned()), dimension]),
    ]);
    let info = Json::Object(Object::from([(
        "attributes".to_owned(),
        Json::List(vec![attribute]),
    )]));
    assert_eq!(extract_index_dim(&info), Some(512));
}

#[rstest]
#[case::no_attributes(Json::Object(Object::new()))]
#[case::no_dimensions_marker(Json::Object(Object::from([(
    "attributes".to_owned(),
    Json::List(vec![vector_attribute_without_marker()]),
)])))]
fn schema_extract_index_dim_is_none_without_the_marker(#[case] info: Json) {
    assert_eq!(extract_index_dim(&info), None);
}

fn vector_attribute_without_marker() -> Json {
    Json::List(vec![
        Json::Text("identifier".to_owned()),
        Json::Text("embedding".to_owned()),
        Json::Text("attribute".to_owned()),
        Json::Text("embedding".to_owned()),
        Json::Text("type".to_owned()),
        Json::Text("VECTOR".to_owned()),
        Json::Text("dim".to_owned()),
        Json::Text("3".to_owned()),
    ])
}

#[test]
fn schema_assert_dim_matches_accepts_the_same_dimension() {
    assert_dim_matches(&info_reply(3), 3, INDEX_NAME).expect("the same dimension");
}

#[test]
fn schema_assert_dim_matches_accepts_an_index_without_a_dimension() {
    let info = Json::Object(Object::new());
    assert_dim_matches(&info, 3, INDEX_NAME).expect("a missing dimension");
}

#[test]
fn schema_assert_dim_matches_fails_with_litellms_text() {
    let Err(error) = assert_dim_matches(&info_reply(4), 3, INDEX_NAME) else {
        panic!("a dimension change must fail");
    };
    assert_eq!(
        error.to_string(),
        format!(
            "Valkey semantic-cache index '{INDEX_NAME}' already exists with embedding dimension 4, \
             but the configured embedding model produced dimension 3. Use a different \
             valkey_semantic_cache_index_name or drop the existing index."
        )
    );
}

#[tokio::test]
async fn index_creates_the_index_on_the_first_write() {
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    cache
        .async_set_cache(
            "user-1",
            &Json::Text("cached".to_owned()),
            &kwargs_with_message(PROMPT),
        )
        .await;
    let creates = index.creates();
    assert_eq!(creates.len(), 1);
    assert_eq!(creates[0].0, INDEX_NAME);
    assert_eq!(
        creates[0].1,
        IndexSchema {
            prefix: format!("{INDEX_NAME}:"),
            dimension: 3,
        }
    );
}

#[tokio::test]
async fn index_remembers_the_dimension_after_a_create() {
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let kwargs = kwargs_with_message(PROMPT);
    cache
        .async_set_cache("user-1", &Json::Text("first".to_owned()), &kwargs)
        .await;
    cache
        .async_set_cache("user-1", &Json::Text("second".to_owned()), &kwargs)
        .await;
    assert_eq!(index.creates().len(), 1);
}

#[tokio::test]
async fn index_a_new_dimension_creates_the_index_again() {
    let embedder = Arc::new(FakeEmbedder::new());
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::clone(&embedder), &index);
    let kwargs = kwargs_with_message(PROMPT);
    cache
        .async_set_cache("user-1", &Json::Text("first".to_owned()), &kwargs)
        .await;
    embedder.set_vector(vec![0.5_f32, 0.5_f32]);
    cache
        .async_set_cache("user-1", &Json::Text("second".to_owned()), &kwargs)
        .await;
    let creates = index.creates();
    assert_eq!(creates.len(), 2);
    assert_eq!(creates[1].1.dimension, 2);
}

#[tokio::test]
async fn index_an_existing_index_with_the_same_dimension_passes() {
    let index = Arc::new(FakeIndex::default().with_info(INDEX_NAME, info_reply(3)));
    index.fail_create_next(exists_error());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    cache
        .async_set_cache(
            "user-1",
            &Json::Text("cached".to_owned()),
            &kwargs_with_message(PROMPT),
        )
        .await;
    assert_eq!(index.stores().len(), 1);
}

#[tokio::test]
async fn index_an_existing_index_with_another_dimension_rejects_the_write() {
    let index = Arc::new(FakeIndex::default().with_info(INDEX_NAME, info_reply(4)));
    index.fail_create_next(exists_error());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let kwargs = kwargs_with_message(PROMPT);
    cache
        .async_set_cache("user-1", &Json::Text("first".to_owned()), &kwargs)
        .await;
    assert!(index.stores().is_empty());
    cache
        .async_set_cache("user-1", &Json::Text("second".to_owned()), &kwargs)
        .await;
    assert_eq!(index.creates().len(), 2);
    assert_eq!(index.stores().len(), 1);
}

#[tokio::test]
async fn index_an_exists_error_is_recognized_in_any_case() {
    let index = Arc::new(FakeIndex::default().with_info(INDEX_NAME, info_reply(3)));
    index.fail_create_next(server_error("INDEX ALREADY EXISTS"));
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    cache
        .async_set_cache(
            "user-1",
            &Json::Text("cached".to_owned()),
            &kwargs_with_message(PROMPT),
        )
        .await;
    assert_eq!(index.stores().len(), 1);
}

#[tokio::test]
async fn index_other_create_errors_are_not_remembered() {
    let index = Arc::new(FakeIndex::default());
    index.fail_create_next(alternative_error());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let kwargs = kwargs_with_message(PROMPT);
    cache
        .async_set_cache("user-1", &Json::Text("first".to_owned()), &kwargs)
        .await;
    assert!(index.stores().is_empty());
    cache
        .async_set_cache("user-1", &Json::Text("second".to_owned()), &kwargs)
        .await;
    assert_eq!(index.creates().len(), 2);
    assert_eq!(index.stores().len(), 1);
}

#[tokio::test]
async fn store_writes_a_document_for_the_prompt() {
    let embedder = Arc::new(FakeEmbedder::new());
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::clone(&embedder), &index);
    cache
        .async_set_cache(
            "user-1",
            &Json::Text("cached".to_owned()),
            &kwargs_with_message(PROMPT),
        )
        .await;
    let stores = index.stores();
    assert_eq!(stores.len(), 1);
    assert_eq!(stores[0].1.prompt, PROMPT);
    assert!(stores[0].2.is_none());
    assert_eq!(embedder.calls()[0].0, PROMPT);
}

#[tokio::test]
async fn store_pipeline_writes_every_pair() {
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let pairs = vec![
        ("user-1".to_owned(), Json::Text("first".to_owned())),
        ("user-2".to_owned(), Json::Text("second".to_owned())),
    ];
    cache
        .async_set_cache_pipeline(&pairs, &kwargs_with_message(PROMPT))
        .await;
    let tags: Vec<String> = index
        .stores()
        .iter()
        .map(|(_, fields, _)| fields.litellm_cache_key.clone())
        .collect();
    assert_eq!(tags.len(), 2);
    assert!(tags.contains(&scope_tag("user-1")));
    assert!(tags.contains(&scope_tag("user-2")));
}

#[rstest]
#[case::no_ttl(None, None)]
#[case::a_ttl(Some(60), Some(60))]
#[case::zero_is_no_expiry(Some(0), None)]
#[tokio::test]
async fn store_uses_the_redis_ttl_rule(#[case] ttl: Option<i64>, #[case] expected: Option<i64>) {
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let mut kwargs = kwargs_with_message(PROMPT);
    kwargs.ttl = ttl;
    cache
        .async_set_cache("user-1", &Json::Text("cached".to_owned()), &kwargs)
        .await;
    assert_eq!(index.stores()[0].2, expected);
}

#[tokio::test]
async fn store_passes_the_request_metadata_to_the_embedder() {
    let embedder = Arc::new(FakeEmbedder::new());
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::clone(&embedder), &index);
    let mut kwargs = kwargs_with_message(PROMPT);
    let metadata = Object::from([("user".to_owned(), Json::Text("u-1".to_owned()))]);
    kwargs.metadata = Some(metadata.clone());
    cache
        .async_set_cache("user-1", &Json::Text("cached".to_owned()), &kwargs)
        .await;
    assert_eq!(embedder.calls()[0].1, Some(metadata));
}

#[tokio::test]
async fn store_stores_the_json_text_of_the_value() {
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let value = Json::Object(Object::from([(
        "answer".to_owned(),
        Json::Text("42".to_owned()),
    )]));
    cache
        .async_set_cache("user-1", &value, &kwargs_with_message(PROMPT))
        .await;
    assert_eq!(index.stores()[0].1.response, "{\"answer\":\"42\"}");
}

#[tokio::test]
async fn store_without_a_prompt_writes_nothing() {
    let embedder = Arc::new(FakeEmbedder::new());
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::clone(&embedder), &index);
    cache
        .async_set_cache(
            "user-1",
            &Json::Text("cached".to_owned()),
            &kwargs_without_prompt(),
        )
        .await;
    assert!(index.stores().is_empty());
    assert!(embedder.calls().is_empty());
}

#[tokio::test]
async fn store_treats_empty_messages_as_no_prompt() {
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let mut kwargs = kwargs_with_message(PROMPT);
    kwargs.messages = Some(Vec::new());
    cache
        .async_set_cache("user-1", &Json::Text("cached".to_owned()), &kwargs)
        .await;
    assert!(index.stores().is_empty());
}

#[tokio::test]
async fn store_swallows_embedding_errors() {
    let embedder = Arc::new(FakeEmbedder::new());
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::clone(&embedder), &index);
    embedder.fail();
    cache
        .async_set_cache(
            "user-1",
            &Json::Text("cached".to_owned()),
            &kwargs_with_message(PROMPT),
        )
        .await;
    assert!(index.stores().is_empty());
}

#[tokio::test]
async fn store_swallows_backend_errors() {
    let index = Arc::new(FakeIndex::default());
    index.fail_store(alternative_error());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    cache
        .async_set_cache(
            "user-1",
            &Json::Text("cached".to_owned()),
            &kwargs_with_message(PROMPT),
        )
        .await;
    assert!(index.stores().is_empty());
}

#[tokio::test]
async fn lookup_returns_the_decoded_response_and_the_similarity() {
    let index = Arc::new(FakeIndex::default());
    index.set_hits(vec![hit(0.25, "{\"text\":\"cached\"}")]);
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let mut metadata = Object::new();
    let value = cache
        .async_get_cache("user-1", &kwargs_with_message(PROMPT), Some(&mut metadata))
        .await;
    assert_eq!(
        value,
        Some(Json::Object(Object::from([(
            "text".to_owned(),
            Json::Text("cached".to_owned())
        )])))
    );
    assert_similarity(&metadata, 0.75_f64);
}

#[tokio::test]
async fn lookup_below_the_threshold_keeps_the_similarity() {
    let index = Arc::new(FakeIndex::default());
    index.set_hits(vec![hit(0.875, "{\"text\":\"cached\"}")]);
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let mut metadata = Object::new();
    let value = cache
        .async_get_cache("user-1", &kwargs_with_message(PROMPT), Some(&mut metadata))
        .await;
    assert!(value.is_none());
    assert_similarity(&metadata, 0.125_f64);
}

#[tokio::test]
async fn lookup_a_similarity_equal_to_the_threshold_is_a_hit() {
    let index = Arc::new(FakeIndex::default());
    index.set_hits(vec![hit(0.75, "{\"text\":\"cached\"}")]);
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let mut metadata = Object::new();
    let value = cache
        .async_get_cache("user-1", &kwargs_with_message(PROMPT), Some(&mut metadata))
        .await;
    assert!(value.is_some());
    assert_similarity(&metadata, 0.25_f64);
}

#[tokio::test]
async fn lookup_writes_zero_similarity_without_a_hit() {
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let mut metadata = Object::new();
    let value = cache
        .async_get_cache("user-1", &kwargs_with_message(PROMPT), Some(&mut metadata))
        .await;
    assert!(value.is_none());
    assert_similarity(&metadata, 0.0_f64);
}

#[tokio::test]
async fn lookup_keeps_the_similarity_when_the_response_does_not_decode() {
    let index = Arc::new(FakeIndex::default());
    index.set_hits(vec![hit(0.25, "not json")]);
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let mut metadata = Object::new();
    let value = cache
        .async_get_cache("user-1", &kwargs_with_message(PROMPT), Some(&mut metadata))
        .await;
    assert!(value.is_none());
    assert_similarity(&metadata, 0.75_f64);
}

#[tokio::test]
async fn lookup_embeds_the_prompt_and_runs_the_knn_query() {
    let embedder = Arc::new(FakeEmbedder::new());
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(Arc::clone(&embedder), &index);
    let metadata = Object::from([("user".to_owned(), Json::Text("u-1".to_owned()))]);
    let mut kwargs = kwargs_with_message(PROMPT);
    kwargs.metadata = Some(metadata.clone());
    cache.async_get_cache("user-1", &kwargs, None).await;
    assert_eq!(embedder.calls()[0].0, PROMPT);
    assert_eq!(embedder.calls()[0].1, Some(metadata));
    let searches = index.searches();
    assert_eq!(searches.len(), 1);
    assert_eq!(searches[0].0, INDEX_NAME);
    assert_eq!(searches[0].1, knn_query("user-1"));
    assert_eq!(searches[0].2, packed_vector());
}

#[tokio::test]
async fn lookup_without_a_prompt_writes_zero_and_skips_the_backend() {
    let embedder = Arc::new(FakeEmbedder::new());
    let index = Arc::new(FakeIndex::default());
    let (cache, connector) = cache_with(Arc::clone(&embedder), &index);
    let mut metadata = Object::new();
    let value = cache
        .async_get_cache("user-1", &kwargs_without_prompt(), Some(&mut metadata))
        .await;
    assert!(value.is_none());
    assert_similarity(&metadata, 0.0_f64);
    assert!(embedder.calls().is_empty());
    assert!(connector.connects().is_empty());
}

#[rstest]
#[case::embedding("embedding")]
#[case::connect("connect")]
#[case::create("create")]
#[case::search("search")]
#[tokio::test]
async fn lookup_errors_become_misses_with_zero_similarity(#[case] failure: &str) {
    let embedder = Arc::new(FakeEmbedder::new());
    let index = Arc::new(FakeIndex::default());
    let (cache, connector) = cache_with(Arc::clone(&embedder), &index);
    match failure {
        "embedding" => embedder.fail(),
        "connect" => connector.fail(alternative_error()),
        "create" => index.fail_create_next(alternative_error()),
        "search" => index.fail_search(alternative_error()),
        _ => panic!("a known failure point"),
    }
    let mut metadata = Object::new();
    let value = cache
        .async_get_cache("user-1", &kwargs_with_message(PROMPT), Some(&mut metadata))
        .await;
    assert!(value.is_none());
    assert_similarity(&metadata, 0.0_f64);
}

#[tokio::test]
async fn lookup_without_metadata_returns_the_hit() {
    let index = Arc::new(FakeIndex::default());
    index.set_hits(vec![hit(0.25, "{\"text\":\"cached\"}")]);
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let value = cache
        .async_get_cache("user-1", &kwargs_with_message(PROMPT), None)
        .await;
    assert!(value.is_some());
}

#[tokio::test]
async fn info_returns_the_ft_info_reply() {
    let index = Arc::new(FakeIndex::default().with_info(INDEX_NAME, info_reply(3)));
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let reply = cache.index_info().await.expect("the FT.INFO reply");
    assert_eq!(reply, info_reply(3));
}

#[tokio::test]
async fn info_opens_the_connector_once_with_the_resolved_url() {
    let index = Arc::new(FakeIndex::default().with_info(INDEX_NAME, info_reply(3)));
    let (cache, connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    cache.index_info().await.expect("the first reply");
    cache.index_info().await.expect("the second reply");
    assert_eq!(connector.connects(), vec!["redis://:pw@localhost:6379"]);
}

#[tokio::test]
async fn info_uses_an_injected_index_without_connecting() {
    let index = Arc::new(FakeIndex::default().with_info(INDEX_NAME, info_reply(3)));
    let (cache, connector) = cache_with_injected_index(&index);
    let reply = cache.index_info().await.expect("the FT.INFO reply");
    assert_eq!(reply, info_reply(3));
    assert!(connector.connects().is_empty());
}

#[tokio::test]
async fn info_propagates_backend_errors() {
    let index = Arc::new(FakeIndex::default());
    index.fail_info(alternative_error());
    let (cache, _connector) = cache_with(Arc::new(FakeEmbedder::new()), &index);
    let result = cache.index_info().await;
    let Err(error) = result else {
        panic!("an FT.INFO failure must surface");
    };
    assert_eq!(error.to_string(), alternative_error().to_string());
}
