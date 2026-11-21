use rstest::rstest;

use crate::legacy_completion_accounting::response_tokens::{
    CompletionResponse, ResponseUsage, response_total_tokens,
};

#[rstest]
#[case::chat_with_usage(CompletionResponse::Chat(Some(ResponseUsage { total_tokens: 42 })), 42)]
#[case::embedding_with_usage(
    CompletionResponse::Embedding(Some(ResponseUsage { total_tokens: 7 })),
    7
)]
#[case::text_completion_with_usage(
    CompletionResponse::TextCompletion(Some(ResponseUsage { total_tokens: 9 })),
    9
)]
#[case::zero_usage(CompletionResponse::Chat(Some(ResponseUsage { total_tokens: 0 })), 0)]
#[case::chat_without_usage(CompletionResponse::Chat(None), 0)]
#[case::embedding_without_usage(CompletionResponse::Embedding(None), 0)]
#[case::text_completion_without_usage(CompletionResponse::TextCompletion(None), 0)]
#[case::other_response(CompletionResponse::Other, 0)]
fn counts_total_tokens_of_counted_responses(
    #[case] response: CompletionResponse,
    #[case] expected: i64,
) {
    assert_eq!(response_total_tokens(&response), expected);
}
