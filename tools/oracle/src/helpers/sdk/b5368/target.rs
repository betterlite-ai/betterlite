use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_error::error::details::{ApiDetails, ResponseDetails};
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_models::decisions::Json;
use betterlite_sdk::call_setup::RequestMap;
use betterlite_sdk::call_setup::correlation::CallOutcome;
use betterlite_sdk::call_wrapper::error::{CallError, plain_error};
use betterlite_sdk::call_wrapper::state::LoggingHandle;
use betterlite_sdk::call_wrapper::target::CallTarget;
use serde_json::{Value, json};

use super::input::{Input, Mode, Step};

// The decorated SDK function: it answers the script in order, and the retry entry is the same stub again
pub(super) struct Target {
    name: &'static str,
    steps: Mutex<VecDeque<Step>>,
    calls: AtomicU64,
    seen: Mutex<Vec<Value>>,
    values: Vec<String>,
    present: Vec<String>,
}

impl Target {
    pub(super) fn new(input: &Input) -> Self {
        let name = match input.mode {
            Mode::Sync => "completion",
            Mode::Async => "acompletion",
        };
        Self {
            name,
            steps: Mutex::new(VecDeque::from(input.outcomes.clone())),
            calls: AtomicU64::new(0),
            seen: Mutex::new(Vec::new()),
            values: input.observe.values.clone(),
            present: input.observe.present.clone(),
        }
    }

    fn record(&self, request: &RequestMap) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let mut values = serde_json::Map::new();
        for key in &self.values {
            let found = request.get(key).cloned().unwrap_or(Json::Null);
            values.insert(key.clone(), value_of(&found));
        }
        let mut present = serde_json::Map::new();
        for key in &self.present {
            present.insert(key.clone(), Value::Bool(request.contains_key(key)));
        }
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({"values": values, "present": present}));
    }

    fn next(&self) -> Result<CallOutcome, CallError> {
        let step = self
            .steps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match step {
            None => Err(CallError::from(plain_error(
                "the stub script has no outcome left",
            ))),
            Some(Step::Response { value }) => Ok(CallOutcome::Response(value)),
            Some(Step::Error {
                error_class,
                message,
            }) => Err(CallError::from(scripted_class(&error_class, &message)?)),
        }
    }

    // The call count and the requests each stub call saw, read together for the report
    pub(super) fn snapshot(&self) -> (u64, Vec<Value>) {
        let calls = self.calls.load(Ordering::Relaxed);
        let seen = self
            .seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        (calls, seen)
    }
}

#[async_trait]
impl CallTarget for Target {
    fn name(&self) -> &str {
        self.name
    }

    async fn call(
        &self,
        request: RequestMap,
        _logging: Option<&LoggingHandle>,
    ) -> Result<CallOutcome, CallError> {
        self.record(&request);
        self.next()
    }

    async fn retry(
        &self,
        request: RequestMap,
        _logging: Option<&LoggingHandle>,
    ) -> Result<CallOutcome, CallError> {
        self.record(&request);
        self.next()
    }
}

fn scripted_class(name: &str, message: &str) -> Result<ErrorClass, CallError> {
    let context = ErrorContext {
        message: message.to_owned(),
        model: None,
        provider: Some("openai".to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    };
    match name {
        "Exception" => Ok(plain_error(message)),
        "APIError" => Ok(ErrorClass::Api(Box::new(ApiDetails {
            context,
            status: 500,
        }))),
        "BadRequestError" => Ok(ErrorClass::BadRequest(Box::new(
            ResponseDetails::with_status(context, None),
        ))),
        "ContextWindowExceededError" => Ok(ErrorClass::ContextWindowExceeded(Box::new(
            ResponseDetails::with_status(context, None),
        ))),
        other => Err(CallError::from(plain_error(&format!(
            "unknown scripted error class {other}"
        )))),
    }
}

pub(super) fn value_of(json: &Json) -> Value {
    match json {
        Json::Null => Value::Null,
        Json::Bool(flag) => Value::Bool(*flag),
        Json::Number(number) => Value::Number(number.clone()),
        Json::Text(text) => Value::String(text.clone()),
        Json::List(items) => Value::Array(items.iter().map(value_of).collect()),
        Json::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), value_of(value)))
                .collect(),
        ),
    }
}
