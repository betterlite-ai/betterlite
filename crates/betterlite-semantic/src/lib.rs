use std::collections::{HashMap, HashSet};
use std::ops::{Div, Mul};

use async_trait::async_trait;
use thiserror::Error;

pub type Vector = Vec<f64>;
pub const DEFAULT_MAX_CACHED_VECTORS: usize = 5000;

#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vector>, EmbedError>;
}

#[derive(Debug, Error)]
pub enum EmbedError {
    #[error("{0}")]
    Http(String),
    #[error("{0}")]
    Other(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmbeddingFailed {
    pub reason: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CacheKey {
    model: String,
    text: String,
}

pub struct SemanticTextIndex {
    max_entries: usize,
    vectors: HashMap<CacheKey, Vector>,
    usage: Vec<CacheKey>,
}

impl SemanticTextIndex {
    #[must_use]
    pub fn new(max_entries: usize) -> Self {
        Self {
            max_entries,
            vectors: HashMap::new(),
            usage: Vec::new(),
        }
    }

    pub async fn scores(
        &mut self,
        query: &str,
        texts: &[String],
        embedder: &dyn Embedder,
        model: &str,
    ) -> Result<Vec<f64>, EmbeddingFailed> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let unique: Vec<String> = texts.iter().cloned().collect::<IndexSet>().0;
        let (query_vector, resolved) = self
            .resolve_vectors(query, &unique, embedder, model)
            .await?;
        if has_dimension_mismatch(&unique, &resolved, query_vector.len()) {
            return Err(EmbeddingFailed {
                reason: format!("embedding model {model} returned vectors of mixed dimensions"),
            });
        }
        self.cache_vectors(&unique, &resolved, model, query_vector.len())?;
        texts
            .iter()
            .map(|text| {
                resolved
                    .get(text)
                    .map(|vector| cosine_similarity(&query_vector, vector))
                    .ok_or_else(|| EmbeddingFailed {
                        reason: format!(
                            "embedding model {model} omitted a vector for a requested text"
                        ),
                    })
            })
            .collect()
    }

    async fn resolve_vectors(
        &self,
        query: &str,
        unique: &[String],
        embedder: &dyn Embedder,
        model: &str,
    ) -> Result<(Vector, HashMap<String, Vector>), EmbeddingFailed> {
        let cached = self.cached_vectors(model);
        let missing = missing_texts(unique, &cached);
        let mut first_batch = vec![query.to_owned()];
        first_batch.extend(missing.iter().cloned());
        let mut batch_vectors = embed_all(embedder, &first_batch).await?;
        let query_vector = take_query_vector(&mut batch_vectors, model)?;
        let mut resolved = cached;
        resolved.extend(missing.into_iter().zip(batch_vectors));
        if has_dimension_mismatch(unique, &resolved, query_vector.len()) {
            let mut retry_batch = vec![query.to_owned()];
            retry_batch.extend(unique.iter().cloned());
            let mut retry_vectors = embed_all(embedder, &retry_batch).await?;
            let retry_query = take_query_vector(&mut retry_vectors, model)?;
            resolved = unique.iter().cloned().zip(retry_vectors).collect();
            return Ok((retry_query, resolved));
        }
        Ok((query_vector, resolved))
    }

    fn cached_vectors(&self, model: &str) -> HashMap<String, Vector> {
        self.vectors
            .iter()
            .filter(|(key, _)| key.model == model)
            .map(|(key, vector)| (key.text.clone(), vector.clone()))
            .collect()
    }
    fn cache_vectors(
        &mut self,
        texts: &[String],
        vectors: &HashMap<String, Vector>,
        model: &str,
        dimension: usize,
    ) -> Result<(), EmbeddingFailed> {
        let touched: HashSet<String> = texts.iter().cloned().collect();
        self.vectors.retain(|key, vector| {
            key.model != model || touched.contains(&key.text) || vector.len() == dimension
        });
        for text in texts {
            let key = CacheKey {
                model: model.to_owned(),
                text: text.clone(),
            };
            let vector = vectors.get(text).ok_or_else(|| EmbeddingFailed {
                reason: format!("embedding model {model} omitted a vector for a requested text"),
            })?;
            self.vectors.insert(key.clone(), vector.clone());
            self.usage.retain(|entry| entry != &key);
            self.usage.push(key);
        }
        while self.vectors.len() > self.max_entries {
            let Some(oldest) = self.usage.first().cloned() else {
                break;
            };
            self.usage.remove(0);
            self.vectors.remove(&oldest);
        }
        Ok(())
    }
}

impl Default for SemanticTextIndex {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_CACHED_VECTORS)
    }
}

mod tests;

struct IndexSet(Vec<String>);
impl FromIterator<String> for IndexSet {
    fn from_iter<T>(iter: T) -> Self
    where
        T: IntoIterator<Item = String>,
    {
        let mut seen = HashSet::new();
        Self(
            iter.into_iter()
                .filter(|text| seen.insert(text.clone()))
                .collect(),
        )
    }
}

fn missing_texts(unique: &[String], cached: &HashMap<String, Vector>) -> Vec<String> {
    unique
        .iter()
        .filter(|text| !cached.contains_key(text.as_str()))
        .cloned()
        .collect()
}

fn take_query_vector(vectors: &mut Vec<Vector>, model: &str) -> Result<Vector, EmbeddingFailed> {
    if vectors.is_empty() {
        return Err(EmbeddingFailed {
            reason: format!("embedding model {model} returned no query vector"),
        });
    }
    Ok(vectors.remove(0))
}

fn has_dimension_mismatch(
    texts: &[String],
    vectors: &HashMap<String, Vector>,
    dimension: usize,
) -> bool {
    texts.iter().any(|text| {
        vectors
            .get(text)
            .is_none_or(|vector| vector.len() != dimension)
    })
}
async fn embed_all(
    embedder: &dyn Embedder,
    texts: &[String],
) -> Result<Vec<Vector>, EmbeddingFailed> {
    match embedder.embed(texts).await {
        Ok(vectors) if vectors.len() == texts.len() => Ok(vectors),
        Ok(vectors) => Err(EmbeddingFailed {
            reason: format!(
                "embedding model returned {} vectors for {} inputs",
                vectors.len(),
                texts.len()
            ),
        }),
        Err(EmbedError::Http(message)) => Err(EmbeddingFailed { reason: message }),
        Err(EmbedError::Other(message)) => Err(EmbeddingFailed {
            reason: format!("embedding the search query failed: {message}"),
        }),
    }
}

#[must_use]
pub fn cosine_similarity(left: &[f64], right: &[f64]) -> f64 {
    let dot = left
        .iter()
        .zip(right)
        .fold(0.0_f64, |sum, (left_value, right_value)| {
            left_value.mul_add(*right_value, sum)
        });
    let left_norm = left.iter().map(|value| value.powi(2)).sum::<f64>().sqrt();
    let right_norm = right.iter().map(|value| value.powi(2)).sum::<f64>().sqrt();
    let denominator = Mul::mul(left_norm, right_norm);
    if denominator == 0.0 {
        0.0
    } else {
        Div::div(dot, denominator)
    }
}
