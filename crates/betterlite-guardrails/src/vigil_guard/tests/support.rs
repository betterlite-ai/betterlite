use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::sync::{Mutex, MutexGuard, PoisonError};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::UnreachableFallback;
use betterlite_guardrails::vigil_guard::DEFAULT_TIMEOUT;
use betterlite_guardrails::vigil_guard::vigil_guard_analysis::{
    Analysis, AnalyzeMode, AnalyzePayload, Source,
};
use betterlite_guardrails::vigil_guard::vigil_guard_config::VigilGuardSettings;
use betterlite_guardrails::vigil_guard::vigil_guard_failure::FailureContext;
use betterlite_guardrails::vigil_guard::vigil_guard_handler_protocol::{
    AsyncPostHandler, HttpResponse, RequestTimeout,
};
use betterlite_guardrails::vigil_guard::vigil_guard_metadata::MetadataValue;
use betterlite_guardrails::vigil_guard::vigil_guard_output::{Inputs, ToolCall};
use betterlite_guardrails::vigil_guard::vigil_guard_transient::TransportError;
use indexmap::IndexMap;
use reqwest::StatusCode;
use reqwest::header::HeaderMap;
use secrecy::SecretString;

#[derive(Debug, Clone)]
pub(crate) struct Call {
    pub(crate) url: String,
    pub(crate) headers: HeaderMap,
    pub(crate) payload: AnalyzePayload,
    pub(crate) timeout: RequestTimeout,
}

pub(crate) struct FakeHandler {
    calls: Mutex<Vec<Call>>,
    responses: Mutex<VecDeque<Result<HttpResponse, TransportError>>>,
}

impl FakeHandler {
    pub(crate) fn new(responses: Vec<Result<HttpResponse, TransportError>>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            responses: Mutex::new(VecDeque::from(responses)),
        }
    }

    pub(crate) fn calls(&self) -> Vec<Call> {
        lock(&self.calls).clone()
    }
}

impl AsyncPostHandler for FakeHandler {
    fn post(
        &self,
        url: &str,
        headers: &HeaderMap,
        json: &AnalyzePayload,
        timeout: RequestTimeout,
    ) -> impl Future<Output = Result<HttpResponse, TransportError>> + Send {
        lock(&self.calls).push(Call {
            url: url.to_owned(),
            headers: headers.clone(),
            payload: json.clone(),
            timeout,
        });
        let response = lock(&self.responses)
            .pop_front()
            .unwrap_or_else(|| Ok(json_body("{}")));
        async move { response }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn json_body(body: &str) -> HttpResponse {
    HttpResponse {
        status: StatusCode::OK,
        body: body.as_bytes().to_vec(),
    }
}

pub(crate) fn status_only(status: StatusCode) -> HttpResponse {
    HttpResponse {
        status,
        body: Vec::new(),
    }
}

pub(crate) fn settings() -> VigilGuardSettings {
    VigilGuardSettings {
        api_base: "https://vigil.example".to_owned(),
        api_key: SecretString::from("sk-vigil-test"),
        timeout: DEFAULT_TIMEOUT,
    }
}

pub(crate) fn payload() -> AnalyzePayload {
    AnalyzePayload {
        text: "hello".to_owned(),
        source: Source::UserInput,
        mode: AnalyzeMode::Full,
        metadata: IndexMap::new(),
    }
}

pub(crate) fn inputs(texts: Option<Vec<String>>, tool_calls: Option<Vec<ToolCall>>) -> Inputs {
    Inputs {
        texts,
        images: None,
        tools: None,
        tool_calls,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    }
}

pub(crate) fn object(json: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(json).expect("valid fixture JSON")
}

pub(crate) fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

pub(crate) fn analysis(json: &str) -> Analysis {
    serde_json::from_str(json).expect("valid analysis JSON")
}

pub(crate) fn tool_call(arguments: &str) -> ToolCall {
    let mut function = BTreeMap::new();
    function.insert("name".to_owned(), text("lookup"));
    function.insert("arguments".to_owned(), text(arguments));
    let mut fields = BTreeMap::new();
    fields.insert("id".to_owned(), text("call_1"));
    fields.insert("type".to_owned(), text("function"));
    fields.insert("index".to_owned(), GuardrailValue::SignedInteger(0));
    fields.insert("function".to_owned(), GuardrailValue::Object(function));
    ToolCall { fields }
}

pub(crate) fn failure_context(fallback: UnreachableFallback) -> FailureContext<'static> {
    FailureContext {
        fallback,
        guardrail_name: "vigil-guard",
        source: Source::UserInput,
    }
}

pub(crate) fn metadata() -> IndexMap<String, MetadataValue> {
    IndexMap::new()
}
