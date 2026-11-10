use crate::cache_key::request::CacheKeyRequest;
use crate::cache_key::value::{KeyObject, KeyValue};

pub const SEMANTIC_SIMILARITY: &str = "semantic-similarity";

// LiteLLM passes a dict of messages, input and a metadata copy to the backend lookup, never the whole request.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CacheLookup {
    pub messages: Option<KeyValue>,
    pub input: Option<KeyValue>,
    pub metadata: Option<KeyObject>,
}

impl CacheLookup {
    #[must_use]
    pub fn project(request: &CacheKeyRequest) -> Self {
        let mut lookup = Self {
            messages: None,
            input: None,
            metadata: request.metadata.clone(),
        };
        for param in &request.params {
            match param.name.as_str() {
                "messages" => lookup.messages = Some(param.value.clone()),
                "input" => lookup.input = Some(param.value.clone()),
                _ => {}
            }
        }
        lookup
    }
}

// LiteLLM copies only the similarity a Python backend stamped into the lookup metadata back onto the request.
pub fn update_metadata(request: &mut CacheKeyRequest, lookup: &CacheLookup) {
    let Some(metadata) = request.metadata.as_mut() else {
        return;
    };
    let Some(lookup_metadata) = lookup.metadata.as_ref() else {
        return;
    };
    let Some(similarity) = lookup_metadata.get(SEMANTIC_SIMILARITY) else {
        return;
    };
    metadata.insert(SEMANTIC_SIMILARITY.to_owned(), similarity.clone());
}

// _stamp_semantic_similarity writes a native lookup's similarity where the Python backends put it.
pub fn stamp_semantic_similarity(request: &mut CacheKeyRequest, similarity: Option<f64>) {
    let Some(found) = similarity else {
        return;
    };
    let Some(metadata) = request.metadata.as_mut() else {
        return;
    };
    if let Some(number) = serde_json::Number::from_f64(found) {
        metadata.insert(SEMANTIC_SIMILARITY.to_owned(), KeyValue::Number(number));
    }
}
