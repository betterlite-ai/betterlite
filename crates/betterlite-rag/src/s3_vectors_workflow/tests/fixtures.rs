use std::collections::BTreeMap;

use betterlite_config::rag::document::ingest::S3VectorsVectorStoreOptions;

use super::support::Call;
use crate::bedrock_ingestion::client::AwsRagResponse;
use crate::s3_vectors_ingestion::{S3VectorData, S3VectorEntry};
use crate::s3_vectors_workflow::settings::S3VectorsSettings;

pub(super) fn options(text: &str) -> S3VectorsVectorStoreOptions {
    serde_json::from_str(text).unwrap()
}

pub(super) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn s3_settings(text: &str) -> S3VectorsSettings {
    S3VectorsSettings::from_vector_store_config(&options(text), None, &BTreeMap::new()).unwrap()
}

pub(super) fn s3_settings_in(text: &str, pairs: &[(&str, &str)]) -> S3VectorsSettings {
    S3VectorsSettings::from_vector_store_config(&options(text), None, &environment(pairs)).unwrap()
}

pub(super) fn bucket_call(name: &str) -> Call {
    Call::Bucket(name.to_owned())
}

pub(super) fn index_call(bucket_name: &str, index_name: &str, dimension: i64) -> Call {
    Call::Index {
        bucket_name: bucket_name.to_owned(),
        index_name: index_name.to_owned(),
        dimension,
        distance_metric: "cosine".to_owned(),
        non_filterable_metadata_keys: vec!["source_text".to_owned()],
    }
}

pub(super) fn entry(
    key: &str,
    text: &str,
    index: &str,
    filename: Option<&str>,
    vector: Vec<f64>,
) -> S3VectorEntry {
    let mut metadata = BTreeMap::from([
        ("source_text".to_owned(), text.to_owned()),
        ("chunk_index".to_owned(), index.to_owned()),
    ]);
    if let Some(filename) = filename {
        metadata.insert("filename".to_owned(), filename.to_owned());
    }
    S3VectorEntry {
        key: key.to_owned(),
        data: S3VectorData { float32: vector },
        metadata,
    }
}

pub(super) fn not_found() -> AwsRagResponse {
    AwsRagResponse {
        status: 404,
        body: br#"{"__type":"NotFoundException"}"#.to_vec(),
    }
}

pub(super) fn accepted() -> AwsRagResponse {
    AwsRagResponse {
        status: 200,
        body: Vec::new(),
    }
}
