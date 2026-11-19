use std::sync::Arc;

use async_trait::async_trait;
use betterlite_core::cost::prepare::BillingTokenCounter;
use betterlite_models::decisions::Json;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::{TextInput, TokenRequest};
use betterlite_tokenizer::reservation::count_input_tokens_for_model;
use indexmap::IndexMap;

// completion_cost counts the request's messages with LiteLLM's token_counter, which the reservation port implements.
pub struct TokenizerCounter {
    pub counter: Arc<Counter>,
}

#[async_trait]
impl BillingTokenCounter for TokenizerCounter {
    async fn count_messages(&self, model: &str, messages: &[Json]) -> Result<usize, CountError> {
        let mut body = IndexMap::with_capacity(1);
        body.insert("messages".to_owned(), Json::List(messages.to_vec()));
        count_input_tokens_for_model(self.counter.as_ref(), &body, model)
            .await
            .ok_or(CountError::MissingInput)
    }

    async fn count_text(&self, model: &str, text: &str) -> Result<usize, CountError> {
        if text.is_empty() {
            return Ok(0);
        }
        let input = TextInput::One(text.to_owned());
        let request = TokenRequest {
            model,
            custom_tokenizer: None,
            text: Some(&input),
            messages: None,
            count_response_tokens: Some(false),
            tools: None,
            tool_choice: None,
        };
        self.counter.count(&request).await
    }
}
