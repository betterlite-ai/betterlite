use betterlite_config::sections::settings::JsonValue;
use betterlite_tokenizer::engine::selection::Selector;
use indexmap::IndexMap;

use self::error::EmbeddingInputError;

pub mod error;
mod tests;

const EMBEDDING_METADATA_FLAG: &str = "semantic-cache-embedding";

// LiteLLM keeps this map in insert order and overwrites a caller key in place
pub type MetadataValue = JsonValue;
pub type MetadataMap = IndexMap<String, MetadataValue>;

pub struct ModelListEntry {
    pub model_name: Option<String>,
}

pub trait ConfiguredTokenLimits {
    fn get_configured_token_limits(&self, model_name: &str) -> (Option<i64>, Option<i64>);
}

#[must_use]
pub fn resolve_embedding_router<'router, R>(
    embedding_model: &str,
    llm_router: Option<&'router R>,
    llm_model_list: Option<&[ModelListEntry]>,
) -> Option<&'router R> {
    let router = llm_router?;
    let model_list = llm_model_list?;
    let serves_model = model_list
        .iter()
        .filter_map(|entry| entry.model_name.as_deref())
        .any(|model_name| model_name == embedding_model);
    serves_model.then_some(router)
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn build_router_embedding_metadata(request_metadata: Option<&MetadataMap>) -> MetadataMap {
    let mut metadata = request_metadata.cloned().unwrap_or_default();
    metadata.insert(
        EMBEDDING_METADATA_FLAG.to_owned(),
        MetadataValue::Bool(true),
    );
    metadata
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn resolve_embedding_max_input_tokens(
    configured: Option<i64>,
    embedding_model: &str,
    router: Option<&dyn ConfiguredTokenLimits>,
) -> Option<i64> {
    if let Some(limit) = configured {
        return Some(limit);
    }
    let (deployment_limit, _) = router?.get_configured_token_limits(embedding_model);
    deployment_limit
}

#[must_use]
pub const fn resolve_embedding_timeout(configured: Option<f64>, default_seconds: f64) -> f64 {
    let Some(value) = configured else {
        return default_seconds;
    };
    value
}

pub async fn truncate_embedding_input(
    selector: &Selector,
    prompt: &str,
    embedding_model: &str,
    max_input_tokens: Option<i64>,
) -> Result<String, EmbeddingInputError> {
    let Some(limit) = max_input_tokens else {
        return Ok(prompt.to_owned());
    };
    let (engine, _) = selector
        .select(embedding_model, None)
        .await
        .map_err(|source| EmbeddingInputError::Select {
            model: embedding_model.to_owned(),
            source: Box::new(source),
        })?;
    let tokens = engine
        .encode(prompt)
        .map_err(|source| EmbeddingInputError::Encode { source })?;
    if usize::try_from(limit).is_ok_and(|cut| tokens.len() <= cut) {
        return Ok(prompt.to_owned());
    }
    engine
        .decode(kept_prefix(&tokens, limit))
        .map_err(|source| EmbeddingInputError::Decode { source })
}

// Python's tokens[:n] keeps n from the front when n >= 0 and drops -n from the end otherwise
fn kept_prefix(tokens: &[u32], max_input_tokens: i64) -> &[u32] {
    let kept = usize::try_from(max_input_tokens).map_or_else(
        |_| {
            let dropped = usize::try_from(max_input_tokens.unsigned_abs()).unwrap_or(usize::MAX);
            tokens.len().saturating_sub(dropped)
        },
        |limit| limit.min(tokens.len()),
    );
    tokens.get(..kept).unwrap_or(tokens)
}
