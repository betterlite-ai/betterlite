use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub(crate) struct CreateCollectionRequest {
    pub(crate) vectors: VectorParams,
    pub(crate) quantization_config: QuantizationParams,
}

#[derive(Serialize)]
pub(crate) struct VectorParams {
    pub(crate) size: u32,
    pub(crate) distance: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QuantizationParams {
    Binary(BinaryParams),
    Scalar(ScalarParams),
    Product(ProductParams),
}

#[derive(Serialize)]
pub(crate) struct BinaryParams {
    pub(crate) always_ram: bool,
}

#[derive(Serialize)]
pub(crate) struct ScalarParams {
    pub(crate) r#type: &'static str,
    pub(crate) quantile: f64,
    pub(crate) always_ram: bool,
}

#[derive(Serialize)]
pub(crate) struct ProductParams {
    pub(crate) compression: &'static str,
    pub(crate) always_ram: bool,
}

#[derive(Serialize)]
pub(crate) struct PayloadIndexRequest {
    pub(crate) field_name: &'static str,
    pub(crate) field_schema: &'static str,
}

#[derive(Serialize)]
pub(crate) struct PointsRequest {
    pub(crate) points: Vec<Point>,
}

#[derive(Serialize)]
pub(crate) struct Point {
    pub(crate) id: String,
    pub(crate) vector: Vec<f64>,
    pub(crate) payload: PointPayload,
}

#[derive(Serialize)]
pub(crate) struct PointPayload {
    pub(crate) litellm_cache_key: String,
    pub(crate) text: String,
    pub(crate) response: String,
}

#[derive(Serialize)]
pub(crate) struct SearchRequest {
    pub(crate) vector: Vec<f64>,
    pub(crate) params: SearchParams,
    pub(crate) limit: u32,
    pub(crate) with_payload: bool,
    pub(crate) filter: CacheKeyFilter,
}

#[derive(Serialize)]
pub(crate) struct SearchParams {
    pub(crate) quantization: QuantizationRescore,
}

#[derive(Serialize)]
pub(crate) struct QuantizationRescore {
    pub(crate) ignore: bool,
    pub(crate) rescore: bool,
    pub(crate) oversampling: f64,
}

#[derive(Serialize)]
pub(crate) struct CacheKeyFilter {
    pub(crate) must: Vec<CacheKeyCondition>,
}

#[derive(Serialize)]
pub(crate) struct CacheKeyCondition {
    pub(crate) key: &'static str,
    #[serde(rename = "match")]
    pub(crate) matches: MatchValue,
}

#[derive(Serialize)]
pub(crate) struct MatchValue {
    pub(crate) value: String,
}

#[derive(Deserialize)]
pub(crate) struct CollectionExistsResponse {
    pub(crate) result: CollectionExists,
}

#[derive(Deserialize)]
pub(crate) struct CollectionExists {
    pub(crate) exists: bool,
}

#[derive(Deserialize)]
pub(crate) struct CreateCollectionResponse {
    pub(crate) result: bool,
}

#[derive(Deserialize)]
pub(crate) struct SearchResponse {
    // A null result is a miss, but a reply without the key is an error like LiteLLM's KeyError
    #[serde(deserialize_with = "Option::deserialize")]
    pub(crate) result: Option<Vec<SearchHit>>,
}

#[derive(Deserialize)]
pub(crate) struct SearchHit {
    pub(crate) score: f64,
    pub(crate) payload: SearchPayload,
}

#[derive(Deserialize)]
pub(crate) struct SearchPayload {
    #[serde(default)]
    pub(crate) litellm_cache_key: Option<String>,
    #[serde(default)]
    pub(crate) text: Option<String>,
    #[serde(default)]
    pub(crate) response: Option<String>,
}
