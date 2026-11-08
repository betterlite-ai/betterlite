use sha2::{Digest as _, Sha256};

use super::error::ValkeySemanticError;
use super::ports::{DocMapping, KnnQuery};
use crate::arguments::json::Json;
use crate::redis_semantic_cache::CACHE_KEY_FIELD_NAME;

pub(crate) const EMBEDDING_FIELD_NAME: &str = "embedding";
pub(crate) const PROMPT_FIELD_NAME: &str = "prompt";
pub(crate) const RESPONSE_FIELD_NAME: &str = "response";
pub(crate) const DISTANCE_FIELD_NAME: &str = "vector_distance";
pub(crate) const VECTOR_PARAM_NAME: &str = "vec";

pub(crate) fn scope_tag(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hex::encode(hasher.finalize())
}

pub(crate) fn pack_vector(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for value in vector {
        bytes.extend(value.to_le_bytes());
    }
    bytes
}

pub(crate) fn doc_mapping(
    key: &str,
    prompt: &str,
    response: &str,
    embedding: &[f32],
) -> DocMapping {
    DocMapping {
        litellm_cache_key: scope_tag(key),
        prompt: prompt.to_owned(),
        response: response.to_owned(),
        embedding: pack_vector(embedding),
    }
}

pub(crate) fn knn_query(key: &str) -> KnnQuery {
    let scope = scope_tag(key);
    KnnQuery {
        text: format!(
            "(@{CACHE_KEY_FIELD_NAME}:{{{scope}}})=>[KNN 1 @{EMBEDDING_FIELD_NAME} \
             $vec AS {DISTANCE_FIELD_NAME}]"
        ),
        return_fields: vec![RESPONSE_FIELD_NAME, DISTANCE_FIELD_NAME],
        dialect: 2,
    }
}

pub(crate) fn extract_index_dim(info: &Json) -> Option<usize> {
    let Json::Object(object) = info else {
        return None;
    };
    let Some(Json::List(attributes)) = object.get("attributes") else {
        return None;
    };
    for attribute in attributes {
        let Json::List(items) = attribute else {
            continue;
        };
        let mut flat = Vec::new();
        for item in items {
            if let Json::List(nested) = item {
                flat.extend(nested.iter());
            } else {
                flat.push(item);
            }
        }
        let Some(marker) = flat
            .iter()
            .position(|item| item.as_str() == Some("dimensions"))
        else {
            continue;
        };
        let value = flat.get(marker.checked_add(1)?)?;
        return dimension_of(value);
    }
    None
}

fn dimension_of(value: &Json) -> Option<usize> {
    match value {
        Json::Text(text) => text.parse().ok(),
        Json::Number(number) => {
            let raw = number.as_u64()?;
            usize::try_from(raw).ok()
        }
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    }
}

pub(crate) fn assert_dim_matches(
    info: &Json,
    dimension: usize,
    index_name: &str,
) -> Result<(), ValkeySemanticError> {
    let Some(existing) = extract_index_dim(info) else {
        return Ok(());
    };
    if existing == dimension {
        return Ok(());
    }
    Err(ValkeySemanticError::IndexDimensionMismatch {
        index: index_name.to_owned(),
        existing,
        configured: dimension,
    })
}
