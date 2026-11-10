use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use rstest::rstest;
use serde::Deserialize;
use tokio::runtime::Builder;

use super::fixtures::{
    ConstantEmbedder, FakeQdrant, Reply, body_of, field, fixed_uuids, json, similarity_of,
};
use crate::arguments::json::{Json, Object};
use crate::message_content::Message;
use crate::qdrant_semantic_cache::options::{QdrantEnv, QdrantSemanticOptions};
use crate::qdrant_semantic_cache::ports::{QdrantReply, QdrantTransport};
use crate::qdrant_semantic_cache::{QdrantCacheKwargs, QdrantSemanticCache};

const DEFAULT_API_BASE: &str = "http://qdrant.test";

#[derive(Deserialize)]
struct CaseFile {
    name: String,
    call: BTreeMap<String, CaseInput>,
}

#[derive(Deserialize)]
struct CaseInput {
    key: String,
    collection: String,
    threshold: f64,
    vector: Vec<f64>,
    messages: Vec<Message>,
    #[serde(default)]
    metadata: Option<Object>,
    #[serde(default = "default_api_base")]
    api_base: String,
    responses: Vec<ScriptedResponse>,
}

#[derive(Deserialize)]
struct ScriptedResponse {
    status: u16,
    body: Json,
}

fn default_api_base() -> String {
    DEFAULT_API_BASE.to_owned()
}

fn replies(input: &CaseInput) -> Vec<Reply> {
    input
        .responses
        .iter()
        .map(|response| {
            Reply::Answer(QdrantReply {
                status: response.status,
                body: serde_json::to_vec(&response.body).expect("a serialized response body"),
            })
        })
        .collect()
}

fn cache_for(input: &CaseInput, transport: &Arc<FakeQdrant>) -> QdrantSemanticCache {
    let options = QdrantSemanticOptions {
        collection_name: Some(input.collection.clone()),
        similarity_threshold: Some(input.threshold),
        qdrant_api_base: Some(input.api_base.clone()),
        qdrant_api_key: None,
        quantization_config: None,
        embedding_model: None,
        vector_size: None,
        scalar_quantile: None,
        embedding_max_input_tokens: None,
        embedding_timeout: None,
    };
    let concrete = Arc::clone(transport);
    let backend: Arc<dyn QdrantTransport> = concrete;
    QdrantSemanticCache::new(
        options,
        &QdrantEnv::default(),
        Arc::new(ConstantEmbedder::with_vector(input.vector.clone())),
        backend,
        Arc::new(fixed_uuids()),
    )
    .expect("a cache built from the case")
}

fn expectations(name: &str) -> (Option<Json>, Option<f64>) {
    match name {
        "qdrant-semantic-cache-search-single-result" => (
            Some(json(r#"{"choices":[{"message":{"content":"hi"}}]}"#)),
            Some(0.93_f64),
        ),
        "qdrant-semantic-cache-search-key-isolation"
        | "qdrant-semantic-cache-search-no-matching-key" => (None, Some(0.0_f64)),
        "qdrant-semantic-cache-search-similarity-threshold" => (None, Some(0.42_f64)),
        "qdrant-semantic-cache-search-quantization-rescore" => {
            (Some(json(r#"{"choices":[]}"#)), None)
        }
        other => panic!("the case {other} has no expected outcome in the replay"),
    }
}

#[rstest]
fn replays_the_qdrant_semantic_search_corpus(
    #[files("../../tools/oracle/cases/helpers/qdrant-semantic-cache/*.yaml")] path: PathBuf,
) {
    let source = fs::read_to_string(&path).expect("the case file");
    let case: CaseFile = serde_saphyr::from_str(&source).expect("a parsed case file");
    let input = case.call.get("b4383").expect("the b4383 call");
    let transport = Arc::new(FakeQdrant::new(0.9_f64, replies(input)));
    let cache = cache_for(input, &transport);
    let runtime = Builder::new_current_thread()
        .build()
        .expect("a test runtime");
    let (value, metadata) = runtime.block_on(async {
        cache.initialize().await.expect("the collection setup");
        let kwargs = QdrantCacheKwargs {
            messages: Some(input.messages.clone()),
            metadata: input.metadata.clone(),
        };
        let mut metadata = input.metadata.clone();
        let value = cache
            .async_get_cache(&input.key, &kwargs, metadata.as_mut())
            .await
            .expect("the search");
        (value, metadata)
    });
    let (expected_value, expected_similarity) = expectations(&case.name);
    assert_eq!(value, expected_value);
    let similarity = metadata.as_ref().and_then(similarity_of);
    assert_eq!(similarity, expected_similarity);
    let search = transport
        .requests()
        .into_iter()
        .find(|request| request.path.ends_with("/points/search"))
        .expect("the search request");
    assert_eq!(
        search.path,
        format!("/collections/{}/points/search", input.collection)
    );
    let body = body_of(&search);
    assert_eq!(field(&body, "limit"), &json("1"));
    assert_eq!(field(&body, "with_payload"), &json("true"));
    assert_eq!(
        field(field(&body, "params"), "quantization"),
        &json(r#"{"ignore":false,"rescore":true,"oversampling":3.0}"#)
    );
    assert_eq!(
        field(&body, "filter"),
        &json(&format!(
            r#"{{"must":[{{"key":"litellm_cache_key","match":{{"value":"{}"}}}}]}}"#,
            input.key
        ))
    );
    let vector = Json::List(
        input
            .vector
            .iter()
            .map(|component| {
                Json::Number(serde_json::Number::from_f64(*component).expect("a finite vector"))
            })
            .collect(),
    );
    assert_eq!(field(&body, "vector"), &vector);
}
