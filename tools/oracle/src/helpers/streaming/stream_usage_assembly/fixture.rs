use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use betterlite_streaming::stream_usage_assembly::UsageTokenCounter;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct CounterFixture {
    pub(crate) text: i64,
    pub(crate) messages: i64,
    pub(crate) fail_text: bool,
    pub(crate) fail_messages: bool,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum CounterCall {
    Text {
        text: String,
        count_response_tokens: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    Messages {
        model: String,
        messages: Option<Vec<Value>>,
    },
}

pub(crate) struct FixtureCounter {
    fixture: CounterFixture,
    raw_messages: Option<Vec<Value>>,
    calls: Mutex<Vec<CounterCall>>,
}

impl FixtureCounter {
    pub(crate) fn new(fixture: &CounterFixture, raw_messages: Option<Vec<Value>>) -> Self {
        Self {
            fixture: fixture.clone(),
            raw_messages,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, call: CounterCall) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(call);
        }
    }

    pub(crate) fn calls(&self) -> Value {
        let Ok(calls) = self.calls.lock() else {
            return Value::Array(Vec::new());
        };
        Value::Array(
            calls
                .iter()
                .map(|call| serde_json::to_value(call).unwrap_or(Value::Null))
                .collect(),
        )
    }

    fn record_text(&self, model: Option<&str>, text: &str) -> Result<i64, CountError> {
        self.record(CounterCall::Text {
            text: text.to_owned(),
            count_response_tokens: true,
            model: model.map(str::to_owned),
        });
        counted(self.fixture.fail_text, self.fixture.text)
    }
}

const fn counted(failed: bool, count: i64) -> Result<i64, CountError> {
    if failed {
        return Err(CountError::MissingInput);
    }
    Ok(count)
}

#[async_trait]
impl UsageTokenCounter for FixtureCounter {
    async fn count_response_text(&self, model: &str, text: &str) -> Result<i64, CountError> {
        self.record_text(Some(model), text)
    }

    async fn count_text(&self, text: &str) -> Result<i64, CountError> {
        self.record_text(None, text)
    }

    async fn count_messages(
        &self,
        model: &str,
        messages: Option<&[Message]>,
    ) -> Result<i64, CountError> {
        self.record(CounterCall::Messages {
            model: model.to_owned(),
            messages: self.raw_messages.clone(),
        });
        if messages.is_none() {
            return Err(CountError::MissingInput);
        }
        counted(self.fixture.fail_messages, self.fixture.messages)
    }
}

#[derive(Default)]
pub(crate) struct FixtureLogger {
    records: Mutex<Vec<Payload>>,
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

impl FixtureLogger {
    fn record(&self, payload: &Payload) -> bool {
        if let Ok(mut records) = self.records.lock() {
            records.push(payload.clone());
        }
        true
    }

    pub(crate) fn messages(&self) -> Value {
        match self.records.lock() {
            Ok(records) => Value::Array(records.to_vec()),
            Err(_poisoned) => Value::Array(Vec::new()),
        }
    }
}

#[derive(Default)]
pub(crate) struct FixtureConsole;

impl ConsoleSink for FixtureConsole {
    fn print(&self, _payload: &Payload) -> bool {
        true
    }
}

pub(crate) fn hooks()
-> Result<VerboseHooks<FixtureLogger, FixtureConsole>, Box<crate::helpers::HelperError>> {
    let redactor =
        Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| failure(error.to_string()))?;
    Ok(VerboseHooks {
        logger: FixtureLogger::default(),
        console: FixtureConsole,
        set_verbose: Payload::Bool(false),
        redactor: Arc::new(redactor),
    })
}

pub(crate) fn failure(message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
