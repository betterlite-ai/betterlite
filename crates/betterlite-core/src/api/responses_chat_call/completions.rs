use std::error::Error;
use std::future::Future;

use betterlite_http::error::HttpError;
use futures_util::stream::BoxStream;

use crate::api::responses_chat_output::records::ChatCompletionResponse;

use super::records::CompletionArguments;

// B014's streamed body, which the source parses as provider-sent SSE frames
pub struct CompletionBody(pub BoxStream<'static, Result<Vec<u8>, HttpError>>);

// The three answers litellm.completion can hand the handler; the provider's ModelResponse and its CustomStreamWrapper are the first two
pub enum CompletionOutput {
    Response(Box<ChatCompletionResponse>),
    Stream(CompletionBody),
    Unexpected { python_type: String },
}

// B014's chat completion service behind the one call the handler makes; the caller drops the bridge-only arguments
pub trait CompletionClient {
    type Error: Error + Send + 'static;

    fn completion(
        &self,
        arguments: &CompletionArguments,
    ) -> impl Future<Output = Result<CompletionOutput, Self::Error>> + Send;
}
