use super::{Counter, EmbeddingReply, FakeTransport, TestCredentials, Workspace, runtime};
use rstest::rstest;

#[rstest]
#[case::hugging_face("hf-endpoint", "[[1, 2]]", 2)]
#[case::cohere(
    "cohere-endpoint",
    r#"{"embeddings":[[1,2]],"meta":{"billed_units":{"input_tokens":7}}}"#,
    7
)]
#[tokio::test]
async fn embedding_uses_the_provider_reader(
    #[case] model: &str,
    #[case] body: &str,
    #[case] expected_tokens: u64,
) {
    let counter = Counter;
    let workspace = Workspace::new();
    let reply = runtime(
        FakeTransport::new(200, body, &[]),
        TestCredentials::working(),
        &counter,
    )
    .embedding(&workspace.embedding(model))
    .await
    .expect("embedding reply");
    match reply {
        EmbeddingReply::HuggingFace(response) => {
            assert_eq!(response.model, model);
            assert_eq!(response.data.len(), 1);
            assert_eq!(response.usage.prompt_tokens, expected_tokens);
        }
        EmbeddingReply::Cohere(response) => {
            assert_eq!(response.model, model);
            assert_eq!(response.data.len(), 1);
            assert_eq!(
                response.usage.prompt_tokens,
                i64::try_from(expected_tokens).expect("small count")
            );
        }
        EmbeddingReply::Voyage(_) => panic!("the model must select its own reader"),
    }
}
