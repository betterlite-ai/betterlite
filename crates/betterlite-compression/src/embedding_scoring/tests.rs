#![cfg(test)]

use super::*;
use async_trait::async_trait;
use serde_json::{Map, Value, json};
use std::sync::Mutex;

const MODEL: &str = "text-embedding-3-small";

#[derive(Debug, thiserror::Error)]
enum FakeFailure {
    #[error("the fake embedding call failed")]
    Failed,
}

struct RecordingExecutor {
    calls: Mutex<Vec<Map<String, Value>>>,
    embeddings: Vec<Vec<f64>>,
}

impl RecordingExecutor {
    fn new(embeddings: Vec<Vec<f64>>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            embeddings,
        }
    }

    fn recorded(&self) -> Vec<Map<String, Value>> {
        self.calls.lock().expect("recorded calls").clone()
    }
}

#[async_trait]
impl EmbeddingExecutor for RecordingExecutor {
    type Failure = FakeFailure;

    async fn embed(&self, call: EmbeddingCall<'_>) -> Result<Vec<Vec<f64>>, Self::Failure> {
        self.calls
            .lock()
            .expect("recorded calls")
            .push(call.parameters.clone());
        Ok(self.embeddings.clone())
    }
}

struct FailingExecutor;

#[async_trait]
impl EmbeddingExecutor for FailingExecutor {
    type Failure = FakeFailure;

    async fn embed(&self, _call: EmbeddingCall<'_>) -> Result<Vec<Vec<f64>>, Self::Failure> {
        Err(FakeFailure::Failed)
    }
}

fn close(actual: f64, expected: f64) -> bool {
    expected.mul_add(-1.0, actual).abs() < 1e-9
}

fn request<'request>(
    query: &'request str,
    messages: &'request [Value],
) -> EmbeddingScoringRequest<'request> {
    EmbeddingScoringRequest {
        query,
        messages,
        model: MODEL,
        embedding_model_params: None,
    }
}

async fn run_scorer(
    scoring_request: EmbeddingScoringRequest<'_>,
    executor: &RecordingExecutor,
) -> Vec<f64> {
    embedding_score_messages(scoring_request, None::<&()>, executor)
        .await
        .expect("scores")
}

#[test]
fn keeps_text_up_to_the_limit_unchanged() {
    let text = "a".repeat(30_000);
    assert_eq!(truncate_text(&text), text);
}

#[test]
fn truncates_text_over_the_limit_to_first_and_last_halves() {
    let text = format!("{}{}{}", "x".repeat(15_000), "m", "y".repeat(15_000));
    let truncated = truncate_text(&text);
    assert_eq!(
        truncated,
        format!("{}\n...\n{}", "x".repeat(15_000), "y".repeat(15_000))
    );
    assert_eq!(truncated.chars().count(), 30_005);
}

#[test]
fn truncates_by_characters_not_bytes() {
    let text = "é".repeat(30_001);
    let truncated = truncate_text(&text);
    assert_eq!(truncated.chars().count(), 30_005);
    assert_eq!(truncated.matches('é').count(), 30_000);
}

#[tokio::test]
async fn passes_query_then_message_contents_to_the_executor() {
    let executor = RecordingExecutor::new(vec![vec![1.0, 0.0], vec![1.0, 0.0], vec![0.0, 1.0]]);
    let messages = vec![
        json!({"role": "user", "content": "auth code"}),
        json!({"role": "user", "content": "cooking recipe"}),
    ];
    embedding_score_messages(request("auth", &messages), None::<&()>, &executor)
        .await
        .expect("scores");
    let calls = executor.recorded();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].get("model"), Some(&json!(MODEL)));
    assert_eq!(
        calls[0].get("input"),
        Some(&json!(["auth", "auth code", "cooking recipe"]))
    );
}

#[tokio::test]
async fn replaces_blank_texts_with_the_empty_placeholder() {
    let executor = RecordingExecutor::new(vec![vec![0.0], vec![0.0], vec![0.0]]);
    let messages = vec![json!({"content": "   "}), json!({"content": "real"})];
    embedding_score_messages(request("", &messages), None::<&()>, &executor)
        .await
        .expect("scores");
    assert_eq!(
        executor.recorded()[0].get("input"),
        Some(&json!(["empty", "empty", "real"]))
    );
}

#[tokio::test]
async fn extracts_text_blocks_and_unrolls_tool_results() {
    let executor = RecordingExecutor::new(vec![vec![0.0], vec![0.0]]);
    let messages = vec![json!({
        "content": [
            {"type": "text", "text": "one"},
            {"type": "image_url", "image_url": {"url": "x"}},
            "two",
            {"type": "tool_result", "content": [{"type": "text", "text": "three"}]},
        ],
    })];
    embedding_score_messages(request("q", &messages), None::<&()>, &executor)
        .await
        .expect("scores");
    assert_eq!(
        executor.recorded()[0].get("input"),
        Some(&json!(["q", "one two three"]))
    );
}

#[tokio::test]
async fn enables_caching_only_when_a_cache_is_present() {
    let messages = vec![json!({"content": "auth"})];

    let plain_executor = RecordingExecutor::new(vec![vec![1.0], vec![1.0]]);
    embedding_score_messages(request("auth", &messages), None::<&()>, &plain_executor)
        .await
        .expect("scores");
    assert_eq!(
        plain_executor.recorded()[0].get("caching"),
        Some(&json!(false))
    );

    let cached_executor = RecordingExecutor::new(vec![vec![1.0], vec![1.0]]);
    let cache = ();
    embedding_score_messages(request("auth", &messages), Some(&cache), &cached_executor)
        .await
        .expect("scores");
    assert_eq!(
        cached_executor.recorded()[0].get("caching"),
        Some(&json!(true))
    );
}

#[tokio::test]
async fn forwards_embedding_model_params() {
    let executor = RecordingExecutor::new(vec![vec![1.0], vec![1.0]]);
    let messages = vec![json!({"content": "auth"})];
    let params: Map<String, Value> = Map::from_iter([
        (
            "api_base".to_owned(),
            json!("https://example-embeddings.test"),
        ),
        ("dimensions".to_owned(), json!(256_u32)),
    ]);
    let configured_request = EmbeddingScoringRequest {
        query: "auth",
        messages: &messages,
        model: MODEL,
        embedding_model_params: Some(&params),
    };
    embedding_score_messages(configured_request, None::<&()>, &executor)
        .await
        .expect("scores");
    let calls = executor.recorded();
    assert_eq!(
        calls[0].get("api_base"),
        Some(&json!("https://example-embeddings.test")),
        "every supplied parameter reaches the executor"
    );
    assert_eq!(calls[0].get("dimensions"), Some(&json!(256_u32)));
}

#[tokio::test]
async fn embedding_model_params_override_the_call_values() {
    let executor = RecordingExecutor::new(vec![vec![1.0], vec![1.0]]);
    let messages = vec![json!({"content": "auth"})];
    let params: Map<String, Value> = Map::from_iter([
        ("caching".to_owned(), json!(false)),
        ("model".to_owned(), json!("other-embedding-model")),
    ]);
    let configured_request = EmbeddingScoringRequest {
        query: "auth",
        messages: &messages,
        model: MODEL,
        embedding_model_params: Some(&params),
    };
    let cache = ();
    embedding_score_messages(configured_request, Some(&cache), &executor)
        .await
        .expect("scores");
    let calls = executor.recorded();
    assert_eq!(calls[0].get("caching"), Some(&json!(false)));
    assert_eq!(calls[0].get("model"), Some(&json!("other-embedding-model")));
}

#[tokio::test]
async fn computes_cosine_similarity_per_message() {
    let executor = RecordingExecutor::new(vec![
        vec![1.0, 0.0],
        vec![1.0, 0.0],
        vec![0.0, 1.0],
        vec![1.0, 1.0],
    ]);
    let messages = vec![
        json!({"content": "a"}),
        json!({"content": "b"}),
        json!({"content": "c"}),
    ];
    let scores = run_scorer(request("auth", &messages), &executor).await;
    assert_eq!(scores.len(), 3);
    assert!(close(scores[0], 1.0));
    assert!(close(scores[1], 0.0));
    assert!(close(scores[2], 0.707_106_781_186_547_5));
}

#[tokio::test]
async fn returns_zero_for_zero_norm_vectors() {
    let executor = RecordingExecutor::new(vec![vec![0.0, 0.0], vec![1.0, 0.0], vec![0.0, 0.0]]);
    let messages = vec![json!({"content": "a"}), json!({"content": "b"})];
    let scores = run_scorer(request("q", &messages), &executor).await;
    assert!(close(scores[0], 0.0));
    assert!(close(scores[1], 0.0));
}

#[tokio::test]
async fn compares_dot_products_up_to_the_shorter_vector() {
    let executor = RecordingExecutor::new(vec![vec![1.0, 0.0], vec![1.0], vec![1.0, 0.0, 5.0]]);
    let messages = vec![json!({"content": "a"}), json!({"content": "b"})];
    let scores = run_scorer(request("q", &messages), &executor).await;
    assert!(close(scores[0], 1.0));
    assert!(close(scores[1], 0.196_116_135_138_184_04));
}

#[tokio::test]
async fn truncates_long_contents_before_the_embedding_call() {
    let executor = RecordingExecutor::new(vec![vec![1.0], vec![1.0]]);
    let messages = vec![json!({"content": "z".repeat(30_001)})];
    run_scorer(request("q", &messages), &executor).await;
    let calls = executor.recorded();
    let inputs = calls[0]
        .get("input")
        .and_then(Value::as_array)
        .expect("input array");
    let content = inputs[1].as_str().expect("content text");
    assert_eq!(content.chars().count(), 30_005);
    assert!(content.starts_with(&"z".repeat(15_000)));
    assert!(content.ends_with(&"z".repeat(15_000)));
}

#[tokio::test]
async fn truncates_a_long_query_before_the_embedding_call() {
    let executor = RecordingExecutor::new(vec![vec![1.0], vec![1.0]]);
    let messages = vec![json!({"content": "short"})];
    let query = format!("{}m{}", "a".repeat(15_000), "b".repeat(15_000));
    run_scorer(request(&query, &messages), &executor).await;
    let expected = format!("{}\n...\n{}", "a".repeat(15_000), "b".repeat(15_000));
    assert_eq!(
        executor.recorded()[0].get("input"),
        Some(&json!([expected, "short"]))
    );
}

#[tokio::test]
async fn returns_no_scores_without_messages() {
    let executor = RecordingExecutor::new(vec![vec![1.0, 0.0]]);
    let messages: Vec<Value> = Vec::new();
    let scores = run_scorer(request("q", &messages), &executor).await;
    assert!(scores.is_empty());
    assert_eq!(executor.recorded()[0].get("input"), Some(&json!(["q"])));
}

#[tokio::test]
async fn errors_when_the_response_carries_no_vectors() {
    let executor = RecordingExecutor::new(Vec::new());
    let messages = vec![json!({"content": "a"})];
    let error = embedding_score_messages(request("q", &messages), None::<&()>, &executor)
        .await
        .expect_err("an empty response is an error");
    assert!(matches!(error, EmbeddingScoringError::EmptyResponse));
}

#[tokio::test]
async fn propagates_the_executor_failure() {
    let messages = vec![json!({"content": "a"})];
    let error = embedding_score_messages(request("q", &messages), None::<&()>, &FailingExecutor)
        .await
        .expect_err("an executor failure is an error");
    assert!(matches!(
        error,
        EmbeddingScoringError::Executor(FakeFailure::Failed)
    ));
    assert_eq!(error.to_string(), "the embedding call failed");
}
