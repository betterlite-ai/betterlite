#![cfg(test)]

use std::sync::Mutex;

use super::*;

struct FixedEmbedder {
    calls: Mutex<Vec<Vec<String>>>,
    mixed: bool,
}
#[async_trait]
impl Embedder for FixedEmbedder {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vector>, EmbedError> {
        let mut calls = self.calls.lock().expect("calls lock");
        let first_call = calls.is_empty();
        calls.push(texts.to_vec());
        drop(calls);
        Ok(texts
            .iter()
            .enumerate()
            .map(|(index, text)| {
                if self.mixed && first_call && index == 1 {
                    vec![1.0_f64]
                } else if text == "zero" {
                    vec![0.0_f64, 0.0_f64]
                } else if text == "orthogonal" {
                    vec![0.0_f64, 1.0_f64]
                } else {
                    vec![1.0_f64, 0.0_f64]
                }
            })
            .collect())
    }
}
fn texts(items: &[&str]) -> Vec<String> {
    items.iter().map(|text| (*text).to_owned()).collect()
}

#[test]
fn semantic_similarity_vectors() {
    assert!(
        (cosine_similarity(&[1.0_f64, 0.0_f64], &[1.0_f64, 0.0_f64]) - 1.0_f64).abs()
            < f64::EPSILON
    );
    assert!(cosine_similarity(&[1.0_f64, 0.0_f64], &[0.0_f64, 1.0_f64]).abs() < f64::EPSILON);
    assert!(cosine_similarity(&[0.0_f64, 0.0_f64], &[1.0_f64, 1.0_f64]).abs() < f64::EPSILON);
}
#[tokio::test]
async fn semantic_empty_and_order() {
    let mut index = SemanticTextIndex::new(10);
    let embedder = FixedEmbedder {
        calls: Mutex::new(Vec::new()),
        mixed: false,
    };
    assert!(
        index
            .scores("q", &[], &embedder, "m")
            .await
            .expect("empty")
            .is_empty()
    );
    assert_eq!(
        index
            .scores("q", &texts(&["orthogonal", "parallel"]), &embedder, "m")
            .await
            .expect("scores"),
        vec![0.0_f64, 1.0_f64]
    );
}
#[tokio::test]
async fn semantic_dedup() {
    let mut index = SemanticTextIndex::new(10);
    let embedder = FixedEmbedder {
        calls: Mutex::new(Vec::new()),
        mixed: false,
    };
    assert_eq!(
        index
            .scores("q", &texts(&["parallel", "parallel"]), &embedder, "m")
            .await
            .expect("scores"),
        vec![1.0_f64, 1.0_f64]
    );
    assert_eq!(embedder.calls.lock().expect("calls")[0].len(), 2);
}
#[tokio::test]
async fn semantic_cache_eviction() {
    let mut index = SemanticTextIndex::new(2);
    let embedder = FixedEmbedder {
        calls: Mutex::new(Vec::new()),
        mixed: false,
    };
    index
        .scores("q", &texts(&["a"]), &embedder, "m1")
        .await
        .expect("first");
    index
        .scores("q", &texts(&["b"]), &embedder, "m2")
        .await
        .expect("second");
    index
        .scores("q", &texts(&["a"]), &embedder, "m1")
        .await
        .expect("touch");
    index
        .scores("q", &texts(&["c"]), &embedder, "m2")
        .await
        .expect("third");
    assert_eq!(index.vectors.len(), 2);
    index
        .scores("q", &texts(&["b"]), &embedder, "m2")
        .await
        .expect("evicted");
    assert_eq!(
        embedder
            .calls
            .lock()
            .expect("calls")
            .last()
            .expect("last")
            .len(),
        2
    );
}
#[test]
fn semantic_default_max_entries() {
    assert_eq!(SemanticTextIndex::default().max_entries, 5000);
}
#[tokio::test]
async fn semantic_dimension_retry() {
    let mut index = SemanticTextIndex::new(10);
    let embedder = FixedEmbedder {
        calls: Mutex::new(Vec::new()),
        mixed: true,
    };
    index
        .scores("q", &texts(&["a", "b"]), &embedder, "m")
        .await
        .expect("retry");
    assert_eq!(embedder.calls.lock().expect("calls").len(), 2);
}
#[tokio::test]
async fn semantic_error_count() {
    struct Bad;
    #[async_trait]
    impl Embedder for Bad {
        async fn embed(&self, _: &[String]) -> Result<Vec<Vector>, EmbedError> {
            Ok(Vec::new())
        }
    }
    let error = embed_all(&Bad, &["q".to_owned()])
        .await
        .expect_err("count mismatch");
    assert!(error.reason.contains("0 vectors for 1 inputs"));
}
#[tokio::test]
async fn semantic_batch_reuse() {
    let mut index = SemanticTextIndex::new(10);
    let embedder = FixedEmbedder {
        calls: Mutex::new(Vec::new()),
        mixed: false,
    };
    index
        .scores("q", &texts(&["a", "b"]), &embedder, "m")
        .await
        .expect("first");
    index
        .scores("q", &texts(&["a", "b"]), &embedder, "m")
        .await
        .expect("second");
    let calls = embedder.calls.lock().expect("calls");
    assert_eq!(calls[0], texts(&["q", "a", "b"]));
    assert_eq!(calls[1], texts(&["q"]));
}
