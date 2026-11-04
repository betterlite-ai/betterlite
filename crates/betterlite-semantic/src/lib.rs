use std::collections::{HashMap, HashSet};

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

#[derive(Clone, Debug, PartialEq)]
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
        let missing: Vec<String> = unique
            .iter()
            .filter(|text| {
                !self.vectors.contains_key(&CacheKey {
                    model: model.to_owned(),
                    text: (*text).clone(),
                })
            })
            .cloned()
            .collect();
        let mut batch = vec![query.to_owned()];
        batch.extend(missing.iter().cloned());
        let embedded = embed_all(embedder, &batch).await?;
        let mut query_vector = embedded[0].clone();
        let mut resolved: HashMap<String, Vector> = self
            .vectors
            .iter()
            .filter(|(key, _)| key.model == model)
            .map(|(key, vector)| (key.text.clone(), vector.clone()))
            .collect();
        resolved.extend(missing.iter().cloned().zip(embedded.into_iter().skip(1)));
        if unique
            .iter()
            .any(|text| resolved[text].len() != query_vector.len())
        {
            let mut retry = vec![query.to_owned()];
            retry.extend(unique.iter().cloned());
            let vectors = embed_all(embedder, &retry).await?;
            query_vector = vectors[0].clone();
            resolved = unique
                .iter()
                .cloned()
                .zip(vectors.into_iter().skip(1))
                .collect();
        }
        if unique
            .iter()
            .any(|text| resolved[text].len() != query_vector.len())
        {
            return Err(EmbeddingFailed {
                reason: format!("embedding model {model} returned vectors of mixed dimensions"),
            });
        }
        let touched: HashSet<String> = unique.iter().cloned().collect();
        self.vectors.retain(|key, vector| {
            key.model != model || touched.contains(&key.text) || vector.len() == query_vector.len()
        });
        for text in &unique {
            let key = CacheKey {
                model: model.to_owned(),
                text: text.clone(),
            };
            self.vectors.insert(key.clone(), resolved[text].clone());
            self.usage.retain(|entry| entry != &key);
            self.usage.push(key);
        }
        while self.vectors.len() > self.max_entries {
            if self.usage.is_empty() {
                break;
            }
            let oldest = self.usage.remove(0);
            self.vectors.remove(&oldest);
        }
        Ok(texts
            .iter()
            .map(|text| cosine_similarity(&query_vector, &resolved[text]))
            .collect())
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
    fn from_iter<T: IntoIterator<Item = String>>(iter: T) -> Self {
        let mut seen = HashSet::new();
        Self(
            iter.into_iter()
                .filter(|text| seen.insert(text.clone()))
                .collect(),
        )
    }
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

pub fn cosine_similarity(left: &[f64], right: &[f64]) -> f64 {
    let dot: f64 = left.iter().zip(right).map(|(a, b)| a * b).sum();
    let left_norm: f64 = left.iter().map(|value| value * value).sum::<f64>().sqrt();
    let right_norm: f64 = right.iter().map(|value| value * value).sum::<f64>().sqrt();
    let denominator = left_norm * right_norm;
    if denominator == 0.0 {
        0.0
    } else {
        dot / denominator
    }
}
