use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_core::environment_validation::probe::{ModelProbe, ProbeCompletion};
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};

// A probe that records what it was asked and either works or refuses.
pub(super) struct RecordingProbe {
    fails: bool,
    calls: Mutex<Vec<Value>>,
}

impl RecordingProbe {
    pub(super) const fn new(fails: bool) -> Self {
        Self {
            fails,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn answer(&self, call: Value) -> Result<(), &'static str> {
        let Ok(mut calls) = self.calls.lock() else {
            return Err("the call log is poisoned");
        };
        calls.push(call);
        if self.fails {
            Err("provider refused")
        } else {
            Ok(())
        }
    }

    pub(super) fn recorded(&self) -> Vec<Value> {
        self.calls.lock().map_or_else(
            |poisoned| poisoned.into_inner().clone(),
            |calls| calls.clone(),
        )
    }
}

impl ModelProbe for RecordingProbe {
    type Failure = &'static str;

    fn retrieve_openai_model(
        &self,
        model: &str,
    ) -> impl Future<Output = Result<(), &'static str>> + Send {
        ready(self.answer(json!({"kind": "retrieve", "model": model})))
    }

    fn complete(
        &self,
        request: &ProbeCompletion<'_>,
    ) -> impl Future<Output = Result<(), &'static str>> + Send {
        ready(self.answer(json!({
            "kind": "completion",
            "model": request.model,
            "user_message": request.user_message,
            "api_key": request.api_key.map(|key| key.expose_secret().to_owned()),
            "max_tokens": request.max_tokens,
        })))
    }
}
