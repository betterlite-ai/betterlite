#![cfg(test)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_rag::s3_vectors_workflow::dimension::{EmbedError, RagEmbedder};
use rstest::rstest;

use super::ListEmbedder;

#[derive(Clone)]
struct ScriptedEmbedder {
    calls: Arc<Mutex<Vec<(String, Vec<String>)>>>,
    answer: Arc<Vec<Vec<f64>>>,
    failure: Option<String>,
}

#[async_trait]
impl RagEmbedder for ScriptedEmbedder {
    async fn embed(&self, model: &str, input: Vec<String>) -> Result<Vec<Vec<f64>>, EmbedError> {
        self.calls.lock().unwrap().push((model.to_owned(), input));
        self.failure.as_ref().map_or_else(
            || Ok(self.answer.as_ref().clone()),
            |message| {
                Err(EmbedError::Failed {
                    model: model.to_owned(),
                    message: message.clone(),
                })
            },
        )
    }
}

fn scripted(answer: Vec<Vec<f64>>) -> ScriptedEmbedder {
    ScriptedEmbedder {
        calls: Arc::new(Mutex::new(Vec::new())),
        answer: Arc::new(answer),
        failure: None,
    }
}

#[tokio::test]
async fn list_embedder_prefers_the_router_and_keeps_the_input_order() {
    let router = scripted(vec![vec![3.0_f64], vec![2.0_f64], vec![1.0_f64]]);
    let native = scripted(vec![vec![9.0_f64]]);
    let router_port: Arc<dyn RagEmbedder> = Arc::new(router.clone());
    let embedder = ListEmbedder {
        router: Some(router_port),
        native: Arc::new(native.clone()),
    };
    let vectors = embedder
        .embed(
            "vendor/embed",
            vec!["first".to_owned(), String::new(), "third".to_owned()],
        )
        .await
        .expect("the embeddings");
    assert_eq!(vectors, vec![vec![3.0_f64], vec![2.0_f64], vec![1.0_f64]]);
    let calls = router.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "vendor/embed");
    assert_eq!(
        calls[0].1,
        vec!["first".to_owned(), String::new(), "third".to_owned()]
    );
    assert!(native.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn list_embedder_uses_the_native_function_without_a_router() {
    let native = scripted(vec![vec![0.0_f64], vec![1.0_f64], vec![2.0_f64]]);
    let embedder = ListEmbedder {
        router: None,
        native: Arc::new(native.clone()),
    };
    let vectors = embedder
        .embed(
            "text-embedding-3-small",
            vec!["first".to_owned(), String::new(), "third".to_owned()],
        )
        .await
        .expect("the embeddings");
    assert_eq!(vectors, vec![vec![0.0_f64], vec![1.0_f64], vec![2.0_f64]]);
    let calls = native.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "text-embedding-3-small");
    assert_eq!(
        calls[0].1,
        vec!["first".to_owned(), String::new(), "third".to_owned()]
    );
}

#[rstest]
#[case::too_few(vec![vec![0.0_f64], vec![1.0_f64]])]
#[case::too_many(vec![vec![0.0_f64], vec![1.0_f64], vec![2.0_f64], vec![3.0_f64]])]
#[tokio::test]
async fn list_embedder_rejects_a_vector_count_mismatch(#[case] answer: Vec<Vec<f64>>) {
    let returned = answer.len();
    let embedder = ListEmbedder {
        router: None,
        native: Arc::new(scripted(answer)),
    };
    let error = embedder
        .embed(
            "embed-model",
            vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
        )
        .await
        .expect_err("the count differs");
    assert_eq!(
        error.to_string(),
        format!(
            "the embedding request for 'embed-model' failed: the embedding response returned {returned} vectors for 3 chunks"
        )
    );
}

#[tokio::test]
async fn list_embedder_passes_a_native_failure_through() {
    let native = ScriptedEmbedder {
        calls: Arc::new(Mutex::new(Vec::new())),
        answer: Arc::new(Vec::new()),
        failure: Some("the provider refused".to_owned()),
    };
    let embedder = ListEmbedder {
        router: None,
        native: Arc::new(native),
    };
    let error = embedder
        .embed("embed-model", vec!["a".to_owned()])
        .await
        .expect_err("the backend fails");
    assert!(matches!(error, EmbedError::Failed { .. }));
    assert_eq!(
        error.to_string(),
        "the embedding request for 'embed-model' failed: the provider refused"
    );
}
