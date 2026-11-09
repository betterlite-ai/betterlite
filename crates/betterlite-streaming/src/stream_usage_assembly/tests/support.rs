use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;

use crate::stream_usage_assembly::wire::{
    Chunk, CompletionTokensDetails, PromptTokensDetails, ReasoningChoice, Usage,
};
use crate::stream_usage_assembly::{
    UsageAssemblyError, UsageRequest, UsageTokenCounter, calculate_usage,
};

#[derive(Clone, Copy)]
enum Outcome {
    Count(i64),
    Fail,
}

impl Outcome {
    fn resolved(self) -> Result<i64, CountError> {
        match self {
            Self::Count(count) => Ok(count),
            Self::Fail => Err(CountError::MissingInput),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Call {
    ResponseText {
        model: String,
        text: String,
    },
    Text {
        text: String,
    },
    Messages {
        model: String,
        messages: Option<usize>,
    },
}

pub(super) struct FixtureCounter {
    response_text: Outcome,
    text: Outcome,
    messages: Outcome,
    calls: Mutex<Vec<Call>>,
}

impl FixtureCounter {
    pub(super) fn counting(response_text: i64, text: i64, messages: i64) -> Self {
        Self::new(
            Outcome::Count(response_text),
            Outcome::Count(text),
            Outcome::Count(messages),
        )
    }

    fn new(response_text: Outcome, text: Outcome, messages: Outcome) -> Self {
        Self {
            response_text,
            text,
            messages,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn failing_text() -> Self {
        Self::new(Outcome::Fail, Outcome::Count(0), Outcome::Count(0))
    }

    pub(super) fn failing_messages() -> Self {
        Self::new(Outcome::Count(0), Outcome::Count(0), Outcome::Fail)
    }

    fn record(&self, call: Call) {
        self.calls.lock().expect("fixture call log").push(call);
    }

    pub(super) fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("fixture call log").clone()
    }
}

#[async_trait]
impl UsageTokenCounter for FixtureCounter {
    async fn count_response_text(&self, model: &str, text: &str) -> Result<i64, CountError> {
        self.record(Call::ResponseText {
            model: model.to_owned(),
            text: text.to_owned(),
        });
        self.response_text.resolved()
    }

    async fn count_text(&self, text: &str) -> Result<i64, CountError> {
        self.record(Call::Text {
            text: text.to_owned(),
        });
        self.text.resolved()
    }

    async fn count_messages(
        &self,
        model: &str,
        messages: Option<&[Message]>,
    ) -> Result<i64, CountError> {
        self.record(Call::Messages {
            model: model.to_owned(),
            messages: messages.map(<[Message]>::len),
        });
        if messages.is_none() {
            return Err(CountError::MissingInput);
        }
        self.messages.resolved()
    }
}

#[derive(Default)]
pub(super) struct FixtureLogger {
    records: Mutex<Vec<Payload>>,
}

impl FixtureLogger {
    fn record(&self, payload: &Payload) -> bool {
        self.records
            .lock()
            .expect("fixture verbose log")
            .push(payload.clone());
        true
    }

    pub(super) fn messages(&self) -> Vec<String> {
        self.records
            .lock()
            .expect("fixture verbose log")
            .iter()
            .map(|payload| match payload {
                Payload::String(text) => text.clone(),
                other @ (Payload::Null
                | Payload::Bool(_)
                | Payload::Number(_)
                | Payload::Array(_)
                | Payload::Object(_)) => other.to_string(),
            })
            .collect()
    }
}

impl LoggerSink for FixtureLogger {
    fn debug(&self, payload: &Payload) -> bool {
        self.record(payload)
    }

    fn info(&self, payload: &Payload) -> bool {
        self.record(payload)
    }

    fn error(&self, payload: &Payload) -> bool {
        self.record(payload)
    }

    fn debug_enabled(&self) -> bool {
        false
    }
}

#[derive(Default)]
pub(super) struct FixtureConsole {
    printed: Mutex<Vec<Payload>>,
}

impl ConsoleSink for FixtureConsole {
    fn print(&self, payload: &Payload) -> bool {
        self.printed
            .lock()
            .expect("fixture console log")
            .push(payload.clone());
        true
    }
}

pub(super) struct UsageCase<'fixture> {
    pub(super) counter: &'fixture FixtureCounter,
    pub(super) chunks: &'fixture [Chunk],
    pub(super) messages: Option<&'fixture [Message]>,
    pub(super) reasoning_tokens: Option<i64>,
    pub(super) count_prompt_tokens:
        Option<&'fixture (dyn Fn() -> Result<i64, CountError> + Send + Sync)>,
}

impl UsageCase<'_> {
    pub(super) async fn run(&self) -> (Result<Usage, UsageAssemblyError>, Vec<String>) {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("fixture redactor compiles");
        let hooks = VerboseHooks {
            logger: FixtureLogger::default(),
            console: FixtureConsole::default(),
            set_verbose: Payload::Bool(false),
            redactor: Arc::new(redactor),
        };
        let request = UsageRequest {
            chunks: self.chunks,
            model: "gpt-4o",
            completion_output: "hi",
            messages: self.messages,
            reasoning_tokens: self.reasoning_tokens,
            count_prompt_tokens: self.count_prompt_tokens,
        };
        let result = calculate_usage(self.counter, &hooks, &request).await;
        (result, hooks.logger.messages())
    }
}

pub(super) fn simple_case<'fixture>(
    counter: &'fixture FixtureCounter,
    chunks: &'fixture [Chunk],
) -> UsageCase<'fixture> {
    UsageCase {
        counter,
        chunks,
        messages: None,
        reasoning_tokens: None,
        count_prompt_tokens: None,
    }
}

pub(super) async fn simple_usage(counter: &FixtureCounter, source: &str) -> Usage {
    let parsed = chunks(source);
    simple_case(counter, &parsed)
        .run()
        .await
        .0
        .expect("the fixture usage is calculated")
}

pub(super) fn chunks(source: &str) -> Vec<Chunk> {
    serde_json::from_str(source).expect("fixture chunks parse")
}

pub(super) fn fixture_usage(source: &str) -> Usage {
    serde_json::from_str(source).expect("fixture usage parses")
}

pub(super) fn fixture_details(source: &str) -> PromptTokensDetails {
    serde_json::from_str(source).expect("fixture details parse")
}

pub(super) fn fixture_completion_details(source: &str) -> CompletionTokensDetails {
    serde_json::from_str(source).expect("fixture completion details parse")
}

pub(super) fn fixture_messages() -> Vec<Message> {
    serde_json::from_str(r#"[{"role":"user","content":"hello"}]"#).expect("fixture messages parse")
}

pub(super) fn fixture_choices(source: &str) -> Vec<ReasoningChoice> {
    serde_json::from_str(source).expect("fixture choices parse")
}

pub(super) const TOKEN_COUNTER_FAILED: &str = "token_counter failed, assuming prompt tokens is 0";
