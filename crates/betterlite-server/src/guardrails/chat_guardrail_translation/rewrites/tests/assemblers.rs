use std::future::Future;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use betterlite_streaming::stream_usage_assembly::UsageTokenCounter;
use betterlite_streaming::stream_usage_assembly::wire::Usage;
use betterlite_streaming::streamed_response_assembly::cost::{
    CostError, CostInput, StreamCostCalculator,
};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use rust_decimal::Decimal;

use crate::guardrails::chat_guardrail_translation::extraction::accumulate_string_content::accumulate_by_choice_index;
use crate::guardrails::chat_guardrail_translation::rewrites::error::RewriteError;
use crate::guardrails::chat_guardrail_translation::rewrites::rebuild_ended_stream::ChunkAssembler;
use crate::guardrails::chat_guardrail_translation::rewrites::response::{
    ChatResponse, MessageContent, ResponseChoice, ResponseFunction, ResponseMessage,
    ResponseToolCall,
};

// Answers every rebuild with one choice whose text is the narrowed stream's content, and the same text as the model, so a test sees what the assembler was given.
pub(crate) struct ConcatenatingAssembler {
    with_tool_call: bool,
    without_choices: bool,
    chunk_counts: Mutex<Vec<usize>>,
}

impl ConcatenatingAssembler {
    pub(crate) fn texts() -> Self {
        Self {
            with_tool_call: false,
            without_choices: false,
            chunk_counts: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn texts_and_a_tool_call() -> Self {
        Self {
            with_tool_call: true,
            without_choices: false,
            chunk_counts: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn without_choices() -> Self {
        Self {
            with_tool_call: false,
            without_choices: true,
            chunk_counts: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn chunk_counts(&self) -> Vec<usize> {
        self.chunk_counts.lock().unwrap().clone()
    }
}

impl ChunkAssembler for ConcatenatingAssembler {
    fn assemble(
        &self,
        chunks: &[Object],
    ) -> impl Future<Output = Result<ChatResponse, RewriteError>> + Send {
        self.chunk_counts.lock().unwrap().push(chunks.len());
        let text: String = accumulate_by_choice_index(chunks).into_values().collect();
        let tool_calls = self.with_tool_call.then(|| {
            vec![ResponseToolCall {
                id: Some("call".to_owned()),
                kind: Some("function".to_owned()),
                function: Some(ResponseFunction {
                    name: Some("lookup".to_owned()),
                    arguments: Some(text.clone()),
                }),
            }]
        });
        let choices = if self.without_choices {
            Vec::new()
        } else {
            vec![ResponseChoice {
                index: 0,
                message: ResponseMessage {
                    content: Some(MessageContent::Text(text.clone())),
                    tool_calls,
                },
            }]
        };
        let response = ChatResponse {
            model: Some(text),
            choices,
        };
        async move { Ok(response) }
    }
}

pub(crate) struct ZeroCounter;

#[async_trait]
impl UsageTokenCounter for ZeroCounter {
    async fn count_response_text(&self, _model: &str, _text: &str) -> Result<i64, CountError> {
        Ok(0)
    }

    async fn count_text(&self, _text: &str) -> Result<i64, CountError> {
        Ok(0)
    }

    async fn count_messages(
        &self,
        _model: &str,
        _messages: Option<&[Message]>,
    ) -> Result<i64, CountError> {
        Ok(0)
    }
}

pub(crate) struct NoCosts;

impl StreamCostCalculator for NoCosts {
    fn completion_cost(
        &self,
        _input: &CostInput<'_>,
        _provider: Option<&str>,
    ) -> Result<Option<Decimal>, CostError> {
        Ok(None)
    }

    fn prompt_completion_cost(
        &self,
        _model: &str,
        _usage: &Usage,
    ) -> Result<(Decimal, Decimal), CostError> {
        Ok((Decimal::ZERO, Decimal::ZERO))
    }
}

pub(crate) struct SilentLogger;

impl LoggerSink for SilentLogger {
    fn debug(&self, _payload: &Payload) -> bool {
        true
    }

    fn info(&self, _payload: &Payload) -> bool {
        true
    }

    fn error(&self, _payload: &Payload) -> bool {
        true
    }

    fn debug_enabled(&self) -> bool {
        false
    }
}

pub(crate) struct SilentConsole;

impl ConsoleSink for SilentConsole {
    fn print(&self, _payload: &Payload) -> bool {
        true
    }
}

pub(crate) fn hooks() -> VerboseHooks<SilentLogger, SilentConsole> {
    VerboseHooks {
        logger: SilentLogger,
        console: SilentConsole,
        set_verbose: Payload::Bool(false),
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()),
    }
}
