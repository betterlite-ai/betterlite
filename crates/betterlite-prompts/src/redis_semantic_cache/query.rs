use super::error::RedisSemanticError;
use super::keys::escape_tag_value;
use super::ports::{Hit, SemanticSearchQuery};
use super::schema::pair_entries;
use super::{CACHE_KEY_FIELD_NAME, VECTOR_FIELD_NAME};
use crate::arguments::json::{Json, Object};

impl SemanticSearchQuery {
    pub(crate) fn query_string(&self) -> String {
        format!(
            "@{VECTOR_FIELD_NAME}:[VECTOR_RANGE $distance_threshold $vector]=>{{$YIELD_DISTANCE_AS: vector_distance}} (@{CACHE_KEY_FIELD_NAME}:{{{}}})",
            escape_tag_value(&self.cache_key)
        )
    }
}

pub(crate) fn parse_search_reply(reply: &Json) -> Result<Vec<Hit>, RedisSemanticError> {
    let Json::List(items) = reply else {
        return Err(RedisSemanticError::MalformedSearchReply);
    };
    let Some(Json::Number(_)) = items.first() else {
        return Err(RedisSemanticError::MalformedSearchReply);
    };
    let Some(documents) = items.get(1..) else {
        return Err(RedisSemanticError::MalformedSearchReply);
    };
    let (entries, remainder) = documents.as_chunks::<2>();
    if !remainder.is_empty() {
        return Err(RedisSemanticError::MalformedSearchReply);
    }
    let mut hits = Vec::new();
    for [key, document] in entries {
        let Json::Text(_key) = key else {
            return Err(RedisSemanticError::MalformedSearchReply);
        };
        let Json::List(fields) = document else {
            return Err(RedisSemanticError::MalformedSearchReply);
        };
        let Some(pairs) = pair_entries(fields) else {
            return Err(RedisSemanticError::MalformedSearchReply);
        };
        let object: Object = pairs
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value.clone()))
            .collect();
        hits.push(hit_from_object(&object));
    }
    Ok(hits)
}

fn hit_from_object(fields: &Object) -> Hit {
    let text = |name: &str| fields.get(name).and_then(Json::as_str).map(str::to_owned);
    Hit {
        prompt: text("prompt"),
        response: text("response"),
        litellm_cache_key: text(CACHE_KEY_FIELD_NAME),
        vector_distance: distance_field(fields),
    }
}

fn distance_field(fields: &Object) -> Option<f64> {
    match fields.get("vector_distance") {
        Some(Json::Number(number)) => number.as_f64(),
        Some(Json::Text(text)) => text.parse().ok(),
        _ => None,
    }
}
