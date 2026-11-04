#![cfg(test)]

use std::sync::Mutex;

use super::*;

struct FixedEmbedder {
    calls: Mutex<Vec<Vec<String>>>,
    dimension: Mutex<usize>,
}
#[async_trait]
impl Embedder for FixedEmbedder {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vector>, EmbedError> {
        self.calls.lock().expect("calls lock").push(texts.to_vec());
        let dimension = *self.dimension.lock().expect("dimension lock");
        Ok(texts
            .iter()
            .map(|text| {
                let mut vector = vec![0.0_f64; dimension];
                match text.as_str() {
                    "zero" => {}
                    "orthogonal" | "b" => vector[1] = 1.0_f64,
                    _ => vector[0] = 1.0_f64,
                }
                vector
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
        dimension: Mutex::new(2),
    };
    assert!(
        index
            .scores("q", &[], &embedder, "m")
            .await
            .expect("empty")
            .is_empty()
    );
    assert!(embedder.calls.lock().expect("calls").is_empty());
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
        dimension: Mutex::new(2),
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
async fn semantic_cache_key_scoped_to_model() {
    let mut index = SemanticTextIndex::new(10);
    let embedder = FixedEmbedder {
        calls: Mutex::new(Vec::new()),
        dimension: Mutex::new(2),
    };
    index
        .scores("q", &texts(&["a"]), &embedder, "m1")
        .await
        .expect("first model");
    index
        .scores("q", &texts(&["a"]), &embedder, "m2")
        .await
        .expect("second model");
    let calls = embedder.calls.lock().expect("calls");
    assert_eq!(calls[0], texts(&["q", "a"]));
    assert_eq!(calls[1], texts(&["q", "a"]));
}
#[tokio::test]
async fn semantic_cache_eviction() {
    let mut index = SemanticTextIndex::new(2);
    let embedder = FixedEmbedder {
        calls: Mutex::new(Vec::new()),
        dimension: Mutex::new(2),
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
        dimension: Mutex::new(2),
    };
    index
        .scores("q", &texts(&["a", "b"]), &embedder, "m")
        .await
        .expect("initial cache fill");
    *embedder.dimension.lock().expect("dimension lock") = 3;
    let scores = index
        .scores("q", &texts(&["a", "b"]), &embedder, "m")
        .await
        .expect("retry after dimension shift");
    assert_eq!(scores, vec![1.0_f64, 0.0_f64]);
    assert_eq!(
        embedder.calls.lock().expect("calls").last().expect("last"),
        &texts(&["q", "a", "b"])
    );
}
#[tokio::test]
async fn semantic_mismatch_after_retry_fails() {
    struct AlwaysMixedEmbedder;
    #[async_trait]
    impl Embedder for AlwaysMixedEmbedder {
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vector>, EmbedError> {
            Ok(texts
                .iter()
                .map(|text| {
                    if text == "a" {
                        vec![1.0_f64]
                    } else {
                        vec![1.0_f64, 0.0_f64]
                    }
                })
                .collect())
        }
    }
    let mut index = SemanticTextIndex::new(10);
    let error = index
        .scores("q", &texts(&["a", "b"]), &AlwaysMixedEmbedder, "m")
        .await
        .expect_err("still mixed after retry");
    assert!(
        matches!(error, SemanticError::Failed(EmbeddingFailed { reason }) if reason.contains("mixed dimensions"))
    );
}
#[tokio::test]
async fn semantic_budget_exceeded_becomes_failed() {
    struct BudgetExceeded;
    #[async_trait]
    impl Embedder for BudgetExceeded {
        async fn embed(&self, _: &[String]) -> Result<Vec<Vector>, EmbedError> {
            Err(EmbedError::Other("budget exceeded for key".to_owned()))
        }
    }
    let mut index = SemanticTextIndex::new(10);
    let error = index
        .scores("q", &texts(&["a"]), &BudgetExceeded, "m")
        .await
        .expect_err("non-HTTP error becomes EmbeddingFailed");
    assert!(
        matches!(error, SemanticError::Failed(EmbeddingFailed { reason }) if reason.contains("budget exceeded for key"))
    );
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
    assert!(
        matches!(error, SemanticError::Failed(EmbeddingFailed { reason }) if reason.contains("0 vectors for 1 inputs"))
    );
}

#[tokio::test]
async fn semantic_http_propagates() {
    struct HttpFailure;
    #[async_trait]
    impl Embedder for HttpFailure {
        async fn embed(&self, _: &[String]) -> Result<Vec<Vector>, EmbedError> {
            Err(EmbedError::Http {
                status_code: 429,
                detail: "provider returned HTTP 429".to_owned(),
            })
        }
    }
    let mut index = SemanticTextIndex::new(10);
    let error = index
        .scores("q", &texts(&["a"]), &HttpFailure, "m")
        .await
        .expect_err("HTTP failure propagates");
    assert!(matches!(
        error,
        SemanticError::Http { status_code, detail }
            if status_code == 429 && detail == "provider returned HTTP 429"
    ));
}
#[tokio::test]
async fn semantic_batch_reuse() {
    let mut index = SemanticTextIndex::new(10);
    let embedder = FixedEmbedder {
        calls: Mutex::new(Vec::new()),
        dimension: Mutex::new(2),
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
