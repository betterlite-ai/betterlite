use std::fmt;
use std::future::Future;

use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::TokenRequest;

pub trait TokenCounter {
    type Error: fmt::Display + Send;

    fn count(
        &self,
        request: &TokenRequest<'_>,
    ) -> impl Future<Output = Result<usize, Self::Error>> + Send;
}

impl TokenCounter for Counter {
    type Error = CountError;

    async fn count(&self, request: &TokenRequest<'_>) -> Result<usize, CountError> {
        Self::count(self, request).await
    }
}
