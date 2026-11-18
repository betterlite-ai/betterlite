use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::{TextInput, TokenRequest};
use betterlite_vertex::gemini_embedding_records::usage::TextTokenCounter;
use tokio::runtime::Handle;
use tokio::task::block_in_place;

// the embedContent usage records count text synchronously, so the async tokenizer runs on a thread the runtime lets block
pub(crate) struct RuntimeTextCounter {
    pub(crate) counter: Counter,
}

impl TextTokenCounter for RuntimeTextCounter {
    fn count_text(&self, model: &str, text: &str) -> Result<u32, CountError> {
        let input = TextInput::One(text.to_owned());
        let request = TokenRequest {
            model,
            custom_tokenizer: None,
            text: Some(&input),
            messages: None,
            count_response_tokens: None,
            tools: None,
            tool_choice: None,
        };
        let counted = block_in_place(|| Handle::current().block_on(self.counter.count(&request)))?;
        u32::try_from(counted).map_err(|_conversion| CountError::Overflow)
    }
}
