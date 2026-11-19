use async_trait::async_trait;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::super::messages_fake_stream::MessagesFakeStream;
use super::error::MessagesExecuteError;

// anthropic_messages' named arguments plus the caller's extras, before any hook has run
pub struct MessagesEntryRequest {
    pub max_tokens: i64,
    pub messages: Vec<Json>,
    pub model: String,
    pub metadata: Option<Json>,
    pub stop_sequences: Option<Json>,
    pub stream: Option<bool>,
    pub system: Option<Json>,
    pub temperature: Option<Json>,
    pub thinking: Option<Json>,
    pub tool_choice: Option<Json>,
    pub tools: Option<Vec<Json>>,
    pub top_k: Option<Json>,
    pub top_p: Option<Json>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub client: Option<ProxiedClient>,
    pub custom_llm_provider: Option<String>,
    pub kwargs: IndexMap<String, Json>,
}

// What the entry returns: a response, a stream, or the None an interceptor may answer with
pub enum MessagesEntryResult {
    Response(Json),
    Stream(Box<dyn MessagesExecuteStream>),
    None,
}

// B2435's fake SSE and the composing caller's provider streams share this object-safe view
#[async_trait]
pub trait MessagesExecuteStream: Send {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, MessagesExecuteError>;
}

#[async_trait]
impl MessagesExecuteStream for MessagesFakeStream {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, MessagesExecuteError> {
        let frame = self.next();
        Ok(frame)
    }
}
