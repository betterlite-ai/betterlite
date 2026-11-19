use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use super::super::dispatch::MessagesExecutionContinuation;
use super::super::entry::{MessagesEntryRequest, MessagesEntryResult};
use super::super::error::{MessagesExecuteError, RaisedException};
use super::super::pipeline::MessagesEntryOutcome;
use super::super::seams::{
    CacheControlHook, CacheControlOutcome, CacheControlRequest, InterceptorRequest,
    MessagesEntrySeams, MessagesInterceptor, PreRequestHook, SearchShortCircuit,
};
use crate::api::messages_reasoning_route::seams::{
    ProviderLookup, ProviderResolution, ResolvedTarget,
};

pub(super) const RAW_HISTORY: &str = r#"[{"role":"user","content":[{"type":"text","text":"   "},{"type":"thinking","thinking":""},{"type":"text","text":"hi"}]},{"role":"assistant","content":[{"type":"tool_use","id":"functions.Bash:0","name":"Bash","input":{"command":"ls"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"functions.Bash:0","content":"ok"}]},{"role":"assistant","content":[{"type":"web_search_tool_result","tool_use_id":"s2","content":[{"type":"web_search_result","title":"T","snippet":"S"}]}]}]"#;
pub(super) const SANITIZED_HISTORY: &str = r#"[{"role":"user","content":[{"type":"text","text":"hi"}]},{"role":"assistant","content":[{"type":"tool_use","id":"functions_Bash_0","name":"Bash","input":{"command":"ls"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"functions_Bash_0","content":"ok"}]},{"role":"assistant","content":[{"type":"text","text":"Web search results:\n\nTitle: T\nSnippet: S"}]}]"#;

pub(super) fn json(text: &str) -> Json {
    let value: Json = serde_json::from_str(text).unwrap();
    value
}

pub(super) fn map(text: &str) -> IndexMap<String, Json> {
    let Json::Object(fields) = json(text) else {
        panic!("the fixture is not an object");
    };
    fields
}

fn redactor() -> Arc<Redactor> {
    let inner = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    Arc::new(inner)
}

pub(super) fn request(model: &str, messages: Vec<Json>) -> MessagesEntryRequest {
    MessagesEntryRequest {
        max_tokens: 32,
        messages,
        model: model.to_owned(),
        metadata: None,
        stop_sequences: None,
        stream: None,
        system: None,
        temperature: None,
        thinking: None,
        tool_choice: None,
        tools: None,
        top_k: None,
        top_p: None,
        api_key: None,
        api_base: None,
        client: None,
        custom_llm_provider: None,
        kwargs: IndexMap::new(),
    }
}

fn optional(value: Option<&Json>) -> Json {
    match value {
        Some(found) => found.clone(),
        None => Json::Null,
    }
}

pub(super) struct Harness {
    pub(super) cache: Cache,
    pub(super) resolver: Resolver,
    pub(super) hooks: Vec<Arc<dyn PreRequestHook>>,
    pub(super) searchers: Vec<Arc<dyn SearchShortCircuit>>,
    pub(super) interceptors: Vec<Arc<dyn MessagesInterceptor>>,
}

impl Harness {
    pub(super) fn new(resolver: Resolver) -> Self {
        Self {
            cache: Cache::new(),
            resolver,
            hooks: Vec::new(),
            searchers: Vec::new(),
            interceptors: Vec::new(),
        }
    }

    pub(super) fn seams(&self) -> MessagesEntrySeams<'_> {
        MessagesEntrySeams {
            cache: &self.cache,
            provider_resolution: &self.resolver,
            pre_request_hooks: &self.hooks,
            search_short_circuits: &self.searchers,
            interceptors: &self.interceptors,
            redactor: redactor(),
        }
    }
}

pub(super) struct CacheCall {
    pub(super) messages: Vec<Json>,
    pub(super) system: Option<Json>,
    pub(super) kwargs: IndexMap<String, Json>,
    pub(super) model: String,
    pub(super) provider: Option<String>,
    pub(super) tools: Option<Vec<Json>>,
    pub(super) api_base: Option<String>,
}

pub(super) struct Cache {
    pub(super) calls: Mutex<Vec<CacheCall>>,
    pub(super) replacement_messages: Option<Vec<Json>>,
    pub(super) replacement_system: Option<Json>,
}

impl Cache {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            replacement_messages: None,
            replacement_system: None,
        }
    }
}

impl CacheControlHook for Cache {
    fn maybe_inject_cache_control(
        &self,
        request: CacheControlRequest<'_>,
    ) -> Result<CacheControlOutcome, MessagesExecuteError> {
        let call = CacheCall {
            messages: request.messages.to_vec(),
            system: request.system.cloned(),
            kwargs: request.kwargs.clone(),
            model: request.model.to_owned(),
            provider: request.custom_llm_provider.map(str::to_owned),
            tools: request.tools.map(<[Json]>::to_vec),
            api_base: request.api_base.map(str::to_owned),
        };
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.push(call);
        drop(calls);
        let messages = match &self.replacement_messages {
            Some(replacement) => replacement.clone(),
            None => request.messages.to_vec(),
        };
        let system = match &self.replacement_system {
            Some(replacement) => Some(replacement.clone()),
            None => request.system.cloned(),
        };
        Ok(CacheControlOutcome { messages, system })
    }
}

pub(super) struct Resolver {
    provider: Option<&'static str>,
    pub(super) lookups: Mutex<Vec<Option<String>>>,
}

impl Resolver {
    pub(super) fn new(provider: Option<&'static str>) -> Self {
        Self {
            provider,
            lookups: Mutex::new(Vec::new()),
        }
    }
}

impl ProviderResolution for Resolver {
    fn resolve(&self, lookup: &ProviderLookup<'_>) -> Option<ResolvedTarget> {
        let mut lookups = self.lookups.lock().unwrap_or_else(PoisonError::into_inner);
        lookups.push(lookup.model.map(str::to_owned));
        drop(lookups);
        self.provider.map(|name| ResolvedTarget {
            model: lookup.model.unwrap_or_default().to_owned(),
            provider: name.to_owned(),
            api_base: None,
        })
    }
}

pub(super) enum HookEffect {
    Pass,
    Mutate(&'static str, Json),
    Replace(IndexMap<String, Json>),
    Raise(RaisedException),
}

pub(super) struct HookCall {
    pub(super) model: String,
    pub(super) messages: Vec<Json>,
    pub(super) kwargs: IndexMap<String, Json>,
}

pub(super) struct Hook {
    effect: HookEffect,
    pub(super) calls: Mutex<Vec<HookCall>>,
}

impl Hook {
    pub(super) fn new(effect: HookEffect) -> Self {
        Self {
            effect,
            calls: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl PreRequestHook for Hook {
    fn callback_name(&self) -> &str {
        "scripted"
    }

    async fn pre_request(
        &self,
        model: &str,
        messages: &[Json],
        kwargs: &mut IndexMap<String, Json>,
    ) -> Result<Option<IndexMap<String, Json>>, MessagesExecuteError> {
        let call = HookCall {
            model: model.to_owned(),
            messages: messages.to_vec(),
            kwargs: kwargs.clone(),
        };
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.push(call);
        drop(calls);
        match &self.effect {
            HookEffect::Pass => Ok(None),
            HookEffect::Mutate(key, value) => {
                kwargs.insert((*key).to_owned(), value.clone());
                Ok(None)
            }
            HookEffect::Replace(replacement) => Ok(Some(replacement.clone())),
            HookEffect::Raise(error) => Err(MessagesExecuteError::Raised(error.clone())),
        }
    }
}

pub(super) struct SearchCall {
    pub(super) model: String,
    pub(super) messages: Vec<Json>,
    pub(super) tools: Option<Vec<Json>>,
    pub(super) provider: Option<String>,
    pub(super) kwargs: IndexMap<String, Json>,
}

pub(super) struct Searcher {
    answer: Option<Json>,
    error: Option<RaisedException>,
    pub(super) calls: Mutex<Vec<SearchCall>>,
}

impl Searcher {
    pub(super) fn new(answer: Option<Json>, error: Option<RaisedException>) -> Self {
        Self {
            answer,
            error,
            calls: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl SearchShortCircuit for Searcher {
    fn callback_name(&self) -> &str {
        "scripted-search"
    }

    async fn try_short_circuit_search(
        &self,
        model: &str,
        messages: &[Json],
        tools: Option<&[Json]>,
        provider: Option<&str>,
        kwargs: &IndexMap<String, Json>,
    ) -> Result<Option<Json>, MessagesExecuteError> {
        let call = SearchCall {
            model: model.to_owned(),
            messages: messages.to_vec(),
            tools: tools.map(<[Json]>::to_vec),
            provider: provider.map(str::to_owned),
            kwargs: kwargs.clone(),
        };
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.push(call);
        drop(calls);
        match &self.error {
            Some(error) => Err(MessagesExecuteError::Raised(error.clone())),
            None => Ok(self.answer.clone()),
        }
    }
}

pub(super) enum InterceptorAnswer {
    Response(Json),
    None,
    Raise(RaisedException),
}

pub(super) struct Interceptor {
    can: bool,
    answer: InterceptorAnswer,
    pub(super) can_calls: Mutex<Vec<(Option<Vec<Json>>, Option<String>)>>,
    pub(super) handled: Mutex<Vec<IndexMap<String, Json>>>,
}

impl Interceptor {
    pub(super) fn new(can: bool, answer: InterceptorAnswer) -> Self {
        Self {
            can,
            answer,
            can_calls: Mutex::new(Vec::new()),
            handled: Mutex::new(Vec::new()),
        }
    }
}

fn recorded(request: &InterceptorRequest<'_>) -> IndexMap<String, Json> {
    let mut record = IndexMap::new();
    record.insert("model".to_owned(), Json::Text(request.model.to_owned()));
    record.insert("messages".to_owned(), Json::List(request.messages.to_vec()));
    record.insert(
        "tools".to_owned(),
        match request.tools {
            Some(list) => Json::List(list.to_vec()),
            None => Json::Null,
        },
    );
    record.insert("stream".to_owned(), Json::Bool(request.stream));
    record.insert(
        "max_tokens".to_owned(),
        Json::Number(serde_json::Number::from(request.max_tokens)),
    );
    record.insert(
        "custom_llm_provider".to_owned(),
        match request.custom_llm_provider {
            Some(name) => Json::Text(name.to_owned()),
            None => Json::Null,
        },
    );
    record.insert(
        "api_key".to_owned(),
        match request.api_key {
            Some(secret) => Json::Text(secret.expose_secret().clone()),
            None => Json::Null,
        },
    );
    record.insert(
        "api_base".to_owned(),
        match request.api_base {
            Some(base) => Json::Text(base.to_owned()),
            None => Json::Null,
        },
    );
    record.insert("metadata".to_owned(), optional(request.metadata));
    record.insert(
        "stop_sequences".to_owned(),
        optional(request.stop_sequences),
    );
    record.insert("system".to_owned(), optional(request.system));
    record.insert("temperature".to_owned(), optional(request.temperature));
    record.insert("thinking".to_owned(), optional(request.thinking));
    record.insert("tool_choice".to_owned(), optional(request.tool_choice));
    record.insert("top_k".to_owned(), optional(request.top_k));
    record.insert("top_p".to_owned(), optional(request.top_p));
    record.insert("kwargs".to_owned(), Json::Object(request.kwargs.clone()));
    record
}

#[async_trait]
impl MessagesInterceptor for Interceptor {
    fn can_handle(&self, tools: Option<&[Json]>, provider: Option<&str>) -> bool {
        let mut calls = self
            .can_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        calls.push((tools.map(<[Json]>::to_vec), provider.map(str::to_owned)));
        drop(calls);
        self.can
    }

    async fn handle(
        &self,
        request: &InterceptorRequest<'_>,
    ) -> Result<MessagesEntryResult, MessagesExecuteError> {
        let mut handled = self.handled.lock().unwrap_or_else(PoisonError::into_inner);
        handled.push(recorded(request));
        drop(handled);
        match &self.answer {
            InterceptorAnswer::Response(value) => Ok(MessagesEntryResult::Response(value.clone())),
            InterceptorAnswer::None => Ok(MessagesEntryResult::None),
            InterceptorAnswer::Raise(error) => Err(MessagesExecuteError::Raised(error.clone())),
        }
    }
}

pub(super) fn continuation(outcome: MessagesEntryOutcome) -> MessagesExecutionContinuation {
    match outcome {
        MessagesEntryOutcome::Dispatch(prepared) => *prepared,
        MessagesEntryOutcome::Returned(_) => panic!("the entry returned instead of dispatching"),
    }
}

pub(super) fn returned(outcome: MessagesEntryOutcome) -> MessagesEntryResult {
    match outcome {
        MessagesEntryOutcome::Returned(result) => result,
        MessagesEntryOutcome::Dispatch(_) => panic!("the entry dispatched instead of returning"),
    }
}

pub(super) fn raised() -> RaisedException {
    RaisedException {
        class: "BadRequestError".to_owned(),
        message: "boom".to_owned(),
        status: Some(400),
        kind: None,
        param: None,
        code: None,
    }
}

pub(super) fn failure(
    outcome: Result<MessagesEntryOutcome, MessagesExecuteError>,
) -> MessagesExecuteError {
    match outcome {
        Ok(_) => panic!("the entry succeeded"),
        Err(error) => error,
    }
}
