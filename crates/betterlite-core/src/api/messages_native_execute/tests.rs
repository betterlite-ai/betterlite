#![cfg(test)]

use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::dispatch::{
    MessagesBaseLlmException, MessagesDispatchOutcome, MessagesExecutionContinuation,
};
use super::entry::{MessagesEntryRequest, MessagesEntryResult, MessagesExecuteStream};
use super::error::{MessagesExecuteError, RaisedException};
use super::pipeline::{MessagesEntryOutcome, execute};
use super::seams::{
    CacheControlHook, CacheControlOutcome, CacheControlRequest, InterceptorRequest,
    MessagesEntrySeams, MessagesInterceptor, PreRequestHook, SearchShortCircuit,
};
use crate::api::messages_reasoning_route::seams::{
    ProviderLookup, ProviderResolution, ResolvedTarget,
};

const RAW_HISTORY: &str = r#"[{"role":"user","content":[{"type":"text","text":"   "},{"type":"thinking","thinking":""},{"type":"text","text":"hi"}]},{"role":"assistant","content":[{"type":"tool_use","id":"functions.Bash:0","name":"Bash","input":{"command":"ls"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"functions.Bash:0","content":"ok"}]},{"role":"assistant","content":[{"type":"web_search_tool_result","tool_use_id":"s2","content":[{"type":"web_search_result","title":"T","snippet":"S"}]}]}]"#;
const SANITIZED_HISTORY: &str = r#"[{"role":"user","content":[{"type":"text","text":"hi"}]},{"role":"assistant","content":[{"type":"tool_use","id":"functions_Bash_0","name":"Bash","input":{"command":"ls"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"functions_Bash_0","content":"ok"}]},{"role":"assistant","content":[{"type":"text","text":"Web search results:\n\nTitle: T\nSnippet: S"}]}]"#;

fn json(text: &str) -> Json {
    let value: Json = serde_json::from_str(text).unwrap();
    value
}

fn map(text: &str) -> IndexMap<String, Json> {
    let Json::Object(fields) = json(text) else {
        panic!("the fixture is not an object");
    };
    fields
}

fn redactor() -> Arc<Redactor> {
    let inner = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    Arc::new(inner)
}

fn request(model: &str, messages: Vec<Json>) -> MessagesEntryRequest {
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

struct Harness {
    cache: Cache,
    resolver: Resolver,
    hooks: Vec<Arc<dyn PreRequestHook>>,
    searchers: Vec<Arc<dyn SearchShortCircuit>>,
    interceptors: Vec<Arc<dyn MessagesInterceptor>>,
}

impl Harness {
    fn new(resolver: Resolver) -> Self {
        Self {
            cache: Cache::new(),
            resolver,
            hooks: Vec::new(),
            searchers: Vec::new(),
            interceptors: Vec::new(),
        }
    }

    fn seams(&self) -> MessagesEntrySeams<'_> {
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

struct CacheCall {
    messages: Vec<Json>,
    system: Option<Json>,
    kwargs: IndexMap<String, Json>,
    model: String,
    provider: Option<String>,
    tools: Option<Vec<Json>>,
    api_base: Option<String>,
}

struct Cache {
    calls: Mutex<Vec<CacheCall>>,
    replacement_messages: Option<Vec<Json>>,
    replacement_system: Option<Json>,
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

struct Resolver {
    provider: Option<&'static str>,
    lookups: Mutex<Vec<Option<String>>>,
}

impl Resolver {
    fn new(provider: Option<&'static str>) -> Self {
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

enum HookEffect {
    Pass,
    Mutate(&'static str, Json),
    Replace(IndexMap<String, Json>),
    Raise(RaisedException),
}

struct HookCall {
    model: String,
    messages: Vec<Json>,
    kwargs: IndexMap<String, Json>,
}

struct Hook {
    effect: HookEffect,
    calls: Mutex<Vec<HookCall>>,
}

impl Hook {
    fn new(effect: HookEffect) -> Self {
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

struct SearchCall {
    model: String,
    messages: Vec<Json>,
    tools: Option<Vec<Json>>,
    provider: Option<String>,
    kwargs: IndexMap<String, Json>,
}

struct Searcher {
    answer: Option<Json>,
    error: Option<RaisedException>,
    calls: Mutex<Vec<SearchCall>>,
}

impl Searcher {
    fn new(answer: Option<Json>, error: Option<RaisedException>) -> Self {
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

enum InterceptorAnswer {
    Response(Json),
    None,
    Raise(RaisedException),
}

struct Interceptor {
    can: bool,
    answer: InterceptorAnswer,
    can_calls: Mutex<Vec<(Option<Vec<Json>>, Option<String>)>>,
    handled: Mutex<Vec<IndexMap<String, Json>>>,
}

impl Interceptor {
    fn new(can: bool, answer: InterceptorAnswer) -> Self {
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

fn continuation(outcome: MessagesEntryOutcome) -> MessagesExecutionContinuation {
    match outcome {
        MessagesEntryOutcome::Dispatch(prepared) => *prepared,
        MessagesEntryOutcome::Returned(_) => panic!("the entry returned instead of dispatching"),
    }
}

fn returned(outcome: MessagesEntryOutcome) -> MessagesEntryResult {
    match outcome {
        MessagesEntryOutcome::Returned(result) => result,
        MessagesEntryOutcome::Dispatch(_) => panic!("the entry dispatched instead of returning"),
    }
}

fn raised() -> RaisedException {
    RaisedException {
        class: "BadRequestError".to_owned(),
        message: "boom".to_owned(),
        status: Some(400),
        kind: None,
        param: None,
        code: None,
    }
}

fn failure(outcome: Result<MessagesEntryOutcome, MessagesExecuteError>) -> MessagesExecuteError {
    match outcome {
        Ok(_) => panic!("the entry succeeded"),
        Err(error) => error,
    }
}

#[tokio::test]
async fn sanitize_before_hooks() {
    let hook = Arc::new(Hook::new(HookEffect::Pass));
    let mut harness = Harness::new(Resolver::new(Some("anthropic")));
    harness.hooks.push(hook.clone());
    let seams = harness.seams();
    let tools = vec![json(r#"{"name":"Bash","input_schema":{"type":"object"}}"#)];
    let mut entry = request("claude-sonnet-4-20250514", {
        let raw: Vec<Json> = serde_json::from_str(RAW_HISTORY).unwrap();
        raw
    });
    entry.tools = Some(tools.clone());
    let prepared = continuation(execute(entry, &seams).await.unwrap());
    let expected: Vec<Json> = serde_json::from_str(SANITIZED_HISTORY).unwrap();
    assert_eq!(prepared.dispatch.messages, expected);
    assert_eq!(prepared.dispatch.tools, Some(tools.clone()));
    let cache_calls = harness
        .cache
        .calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(cache_calls.len(), 1_usize);
    assert_eq!(cache_calls[0].messages, expected);
    assert_eq!(cache_calls[0].model, "claude-sonnet-4-20250514");
    assert_eq!(cache_calls[0].provider, None);
    assert_eq!(cache_calls[0].api_base, None);
    assert_eq!(cache_calls[0].tools, Some(tools));
    assert_eq!(cache_calls[0].kwargs.len(), 0_usize);
    drop(cache_calls);
    let hook_calls = hook.calls.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(hook_calls.len(), 1_usize);
    assert_eq!(hook_calls[0].model, "claude-sonnet-4-20250514");
    assert_eq!(hook_calls[0].messages, expected);
    assert_eq!(
        hook_calls[0].kwargs["tools"],
        Json::List(vec![json(
            r#"{"name":"Bash","input_schema":{"type":"object"}}"#
        )])
    );
}

#[tokio::test]
async fn cache_hook_replaces_messages_and_system() {
    let hook = Arc::new(Hook::new(HookEffect::Pass));
    let mut harness = Harness::new(Resolver::new(Some("anthropic")));
    harness.cache.replacement_messages = Some(vec![json(r#"{"role":"user","content":"swapped"}"#)]);
    harness.cache.replacement_system = Some(json(r#""be brief""#));
    harness.hooks.push(hook.clone());
    let seams = harness.seams();
    let mut entry = request(
        "claude-sonnet-4-20250514",
        vec![json(r#"{"role":"user","content":"hi"}"#)],
    );
    entry.system = Some(json(r#""original""#));
    let prepared = continuation(execute(entry, &seams).await.unwrap());
    assert_eq!(
        prepared.dispatch.messages,
        vec![json(r#"{"role":"user","content":"swapped"}"#)]
    );
    assert_eq!(prepared.dispatch.system, Some(json(r#""be brief""#)));
    let hook_calls = hook.calls.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(
        hook_calls[0].messages,
        vec![json(r#"{"role":"user","content":"swapped"}"#)]
    );
    drop(hook_calls);
    let cache_calls = harness
        .cache
        .calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(cache_calls[0].system, Some(json(r#""original""#)));
}

#[tokio::test]
async fn prerequest_hook_order() {
    let first = Arc::new(Hook::new(HookEffect::Mutate("temperature", json("0"))));
    let second = Arc::new(Hook::new(HookEffect::Pass));
    let mut harness = Harness::new(Resolver::new(Some("anthropic")));
    harness.hooks.push(first.clone());
    harness.hooks.push(second.clone());
    let seams = harness.seams();
    let entry = request(
        "claude-sonnet-4-20250514",
        vec![json(r#"{"role":"user","content":"hi"}"#)],
    );
    let prepared = continuation(execute(entry, &seams).await.unwrap());
    let first_calls = first.calls.lock().unwrap_or_else(PoisonError::into_inner);
    let second_calls = second.calls.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(first_calls.len(), 1_usize);
    assert_eq!(second_calls.len(), 1_usize);
    assert_eq!(first_calls[0].model, "claude-sonnet-4-20250514");
    assert_eq!(
        first_calls[0].kwargs["litellm_params"],
        json(r#"{"custom_llm_provider":"anthropic"}"#)
    );
    assert_eq!(first_calls[0].kwargs["tools"], Json::Null);
    assert_eq!(first_calls[0].kwargs["stream"], Json::Bool(false));
    assert_eq!(first_calls[0].kwargs["tool_choice"], Json::Null);
    assert_eq!(
        second_calls[0].kwargs["temperature"],
        json("0"),
        "the first hook's in-place mutation must survive into the second hook"
    );
    drop(first_calls);
    drop(second_calls);
    assert_eq!(prepared.dispatch.temperature, Some(json("0")));
    assert_eq!(
        prepared.dispatch.custom_llm_provider,
        Some("anthropic".to_owned())
    );
    let lookups = harness
        .resolver
        .lookups
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(lookups.len(), 1_usize);
}

#[tokio::test]
async fn empty_hook_list_keeps_named_arguments() {
    let harness = Harness::new(Resolver::new(Some("anthropic")));
    let seams = harness.seams();
    let mut entry = request(
        "claude-sonnet-4-20250514",
        vec![json(r#"{"role":"user","content":"hi"}"#)],
    );
    entry.stream = Some(true);
    entry.tools = Some(vec![json(r#"{"name":"Bash"}"#)]);
    entry.metadata = Some(json(r#"{"user_id":"u1"}"#));
    entry.system = Some(json(r#""sys""#));
    entry.tool_choice = Some(json(r#"{"type":"auto"}"#));
    let prepared = continuation(execute(entry, &seams).await.unwrap());
    assert_eq!(prepared.dispatch.stream, Some(true));
    assert_eq!(
        prepared.dispatch.tools,
        Some(vec![json(r#"{"name":"Bash"}"#)])
    );
    assert_eq!(
        prepared.dispatch.metadata,
        Some(json(r#"{"user_id":"u1"}"#))
    );
    assert_eq!(prepared.dispatch.system, Some(json(r#""sys""#)));
    assert_eq!(
        prepared.dispatch.tool_choice,
        Some(json(r#"{"type":"auto"}"#))
    );
    assert_eq!(
        prepared.dispatch.custom_llm_provider,
        Some("anthropic".to_owned())
    );
}

#[tokio::test]
async fn provider_resolution_failure_is_ignored() {
    let hook = Arc::new(Hook::new(HookEffect::Pass));
    let mut harness = Harness::new(Resolver::new(None));
    harness.hooks.push(hook.clone());
    let seams = harness.seams();
    let entry = request(
        "unknown-model",
        vec![json(r#"{"role":"user","content":"hi"}"#)],
    );
    let prepared = continuation(execute(entry, &seams).await.unwrap());
    assert_eq!(prepared.dispatch.custom_llm_provider, None);
    let hook_calls = hook.calls.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(
        hook_calls[0].kwargs["litellm_params"],
        json(r#"{"custom_llm_provider":null}"#)
    );
}

#[tokio::test]
async fn hook_named_overrides() {
    let hook = Arc::new(Hook::new(HookEffect::Replace(map(
        r#"{"tools":[{"name":"Search"}],"stream":false,"metadata":null,"temperature":0,"top_k":0,"top_p":null,"litellm_params":{"custom_llm_provider":"bedrock"}}"#,
    ))));
    let mut harness = Harness::new(Resolver::new(Some("anthropic")));
    harness.hooks.push(hook.clone());
    let seams = harness.seams();
    let mut entry = request(
        "claude-sonnet-4-20250514",
        vec![json(r#"{"role":"user","content":"hi"}"#)],
    );
    entry.tools = Some(vec![json(r#"{"name":"Bash"}"#)]);
    entry.stream = Some(true);
    entry.metadata = Some(json(r#"{"user_id":"u1"}"#));
    entry.top_p = Some(json("0.5"));
    let prepared = continuation(execute(entry, &seams).await.unwrap());
    let calls = hook.calls.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(
        calls[0].kwargs["tools"],
        Json::List(vec![json(r#"{"name":"Bash"}"#)]),
        "the hook sees the caller's tools before its own replacement"
    );
    assert_eq!(calls[0].kwargs["stream"], Json::Bool(true));
    drop(calls);
    assert_eq!(
        prepared.dispatch.tools,
        Some(vec![json(r#"{"name":"Search"}"#)])
    );
    assert_eq!(prepared.dispatch.stream, Some(false));
    assert_eq!(prepared.dispatch.metadata, None);
    assert_eq!(prepared.dispatch.temperature, Some(json("0")));
    assert_eq!(prepared.dispatch.top_k, Some(json("0")));
    assert_eq!(prepared.dispatch.top_p, None);
    assert_eq!(
        prepared.dispatch.custom_llm_provider,
        Some("bedrock".to_owned())
    );
}

#[tokio::test]
async fn explicit_provider_wins_over_hooks() {
    let hook = Arc::new(Hook::new(HookEffect::Replace(map(
        r#"{"litellm_params":{"custom_llm_provider":"bedrock"}}"#,
    ))));
    let mut harness = Harness::new(Resolver::new(Some("anthropic")));
    harness.hooks.push(hook);
    let seams = harness.seams();
    let mut entry = request(
        "claude-sonnet-4-20250514",
        vec![json(r#"{"role":"user","content":"hi"}"#)],
    );
    entry.custom_llm_provider = Some("vertex".to_owned());
    let prepared = continuation(execute(entry, &seams).await.unwrap());
    assert_eq!(
        prepared.dispatch.custom_llm_provider,
        Some("vertex".to_owned())
    );
}

#[tokio::test]
async fn hook_map_type_errors() {
    let cases = [
        (r#"{"tools":5}"#, "the field tools must be a list"),
        (r#"{"stream":"yes"}"#, "the field stream must be a boolean"),
        (
            r#"{"litellm_params":{"custom_llm_provider":5}}"#,
            "the field litellm_params.custom_llm_provider must be a string",
        ),
        (
            r#"{"litellm_params":null}"#,
            "'NoneType' object has no attribute 'get'",
        ),
        (
            r#"{"litellm_params":"text"}"#,
            "'str' object has no attribute 'get'",
        ),
        (
            r#"{"litellm_params":5}"#,
            "'int' object has no attribute 'get'",
        ),
    ];
    for (replacement, expected) in cases {
        let mut harness = Harness::new(Resolver::new(None));
        harness
            .hooks
            .push(Arc::new(Hook::new(HookEffect::Replace(map(replacement)))));
        let seams = harness.seams();
        let entry = request(
            "claude-sonnet-4-20250514",
            vec![json(r#"{"role":"user","content":"hi"}"#)],
        );
        let error = failure(execute(entry, &seams).await);
        assert_eq!(error.to_string(), expected);
    }
}

#[tokio::test]
async fn duplicate_hook_extras_raise_type_error() {
    for name in [
        "max_tokens",
        "model",
        "api_key",
        "_litellm_messages_presanitized",
    ] {
        let replacement = format!(r#"{{"{name}":1}}"#);
        let mut harness = Harness::new(Resolver::new(Some("anthropic")));
        harness
            .hooks
            .push(Arc::new(Hook::new(HookEffect::Replace(map(&replacement)))));
        let seams = harness.seams();
        let entry = request(
            "claude-sonnet-4-20250514",
            vec![json(r#"{"role":"user","content":"hi"}"#)],
        );
        let error = failure(execute(entry, &seams).await);
        assert_eq!(
            error.to_string(),
            format!("functools.partial() got multiple values for keyword argument '{name}'")
        );
    }
}

#[tokio::test]
async fn original_litellm_params_survives_merge() {
    let hook = Arc::new(Hook::new(HookEffect::Pass));
    let mut harness = Harness::new(Resolver::new(Some("anthropic")));
    harness.hooks.push(hook.clone());
    let seams = harness.seams();
    let mut entry = request(
        "claude-sonnet-4-20250514",
        vec![json(r#"{"role":"user","content":"hi"}"#)],
    );
    entry.kwargs.insert(
        "litellm_params".to_owned(),
        json(r#"{"custom_llm_provider":"openai"}"#),
    );
    let prepared = continuation(execute(entry, &seams).await.unwrap());
    assert_eq!(
        prepared.dispatch.custom_llm_provider,
        Some("openai".to_owned())
    );
    assert_eq!(
        prepared.dispatch.kwargs["litellm_params"],
        json(r#"{"custom_llm_provider":"openai"}"#)
    );
    let lookups = harness
        .resolver
        .lookups
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(lookups.len(), 1_usize);
}

#[tokio::test]
async fn original_stream_survives_websearch_conversion() {
    let response = json(
        r#"{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4-20250514","content":[{"type":"text","text":"searched"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":5,"output_tokens":2}}"#,
    );
    let searcher = Arc::new(Searcher::new(Some(response.clone()), None));
    let interceptor = Arc::new(Interceptor::new(
        true,
        InterceptorAnswer::Response(json(r#"{"handled":true}"#)),
    ));
    let hook = Arc::new(Hook::new(HookEffect::Mutate("stream", Json::Bool(false))));
    let mut harness = Harness::new(Resolver::new(Some("anthropic")));
    harness.hooks.push(hook);
    harness.searchers.push(searcher.clone());
    harness.interceptors.push(interceptor.clone());
    let seams = harness.seams();
    let mut entry = request(
        "claude-sonnet-4-20250514",
        vec![json(r#"{"role":"user","content":"search"}"#)],
    );
    entry.stream = Some(true);
    let result = returned(execute(entry, &seams).await.unwrap());
    let MessagesEntryResult::Stream(mut stream) = result else {
        panic!("the short circuit must answer with B2435's fake SSE");
    };
    let mut frames = Vec::new();
    while let Some(frame) = stream.next_chunk().await.unwrap() {
        frames.push(String::from_utf8(frame).unwrap());
    }
    assert_eq!(frames.len(), 6_usize);
    assert!(frames[0].starts_with("event: message_start\n"));
    assert_eq!(
        searcher
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)[0]
            .kwargs["metadata"],
        Json::Null
    );
    assert_eq!(
        interceptor
            .can_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len(),
        0_usize,
        "a web-search result must skip the interceptor registry"
    );
}

#[tokio::test]
async fn websearch_marker_streams_without_request_stream() {
    let response = json(
        r#"{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4-20250514","content":[{"type":"text","text":"searched"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":5,"output_tokens":2}}"#,
    );
    let searcher = Arc::new(Searcher::new(Some(response), None));
    let mut harness = Harness::new(Resolver::new(Some("anthropic")));
    harness.searchers.push(searcher);
    let seams = harness.seams();
    let mut entry = request(
        "claude-sonnet-4-20250514",
        vec![json(r#"{"role":"user","content":"search"}"#)],
    );
    entry.kwargs.insert(
        "_websearch_interception_converted_stream".to_owned(),
        Json::Bool(true),
    );
    let result = returned(execute(entry, &seams).await.unwrap());
    assert!(matches!(result, MessagesEntryResult::Stream(_)));

    let silent = Arc::new(Searcher::new(Some(json(r#"{"synthetic":true}"#)), None));
    let mut plain = Harness::new(Resolver::new(Some("anthropic")));
    plain.searchers.push(silent);
    let plain_seams = plain.seams();
    let plain_entry = request(
        "claude-sonnet-4-20250514",
        vec![json(r#"{"role":"user","content":"search"}"#)],
    );
    let plain_result = returned(execute(plain_entry, &plain_seams).await.unwrap());
    assert!(matches!(plain_result, MessagesEntryResult::Response(_)));
}

#[tokio::test]
async fn search_result_skips_interceptors() {
    let first = Arc::new(Searcher::new(Some(json("{}")), None));
    let second = Arc::new(Searcher::new(Some(json(r#"{"second":true}"#)), None));
    let interceptor = Arc::new(Interceptor::new(true, InterceptorAnswer::None));
    let mut harness = Harness::new(Resolver::new(Some("anthropic")));
    harness.searchers.push(first.clone());
    harness.searchers.push(second.clone());
    harness.interceptors.push(interceptor.clone());
    let seams = harness.seams();
    let entry = request(
        "claude-sonnet-4-20250514",
        vec![json(r#"{"role":"user","content":"search"}"#)],
    );
    let result = returned(execute(entry, &seams).await.unwrap());
    let MessagesEntryResult::Response(value) = result else {
        panic!("the first non-null search result must be returned");
    };
    assert_eq!(value, json("{}"));
    assert_eq!(
        second
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len(),
        0_usize,
        "the first non-null result ends the search"
    );
    assert_eq!(
        interceptor
            .can_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len(),
        0_usize
    );
}

#[tokio::test]
async fn first_matching_interceptor_wins() {
    let first = Arc::new(Interceptor::new(false, InterceptorAnswer::None));
    let second = Arc::new(Interceptor::new(
        true,
        InterceptorAnswer::Response(json(r#"{"handled":"second"}"#)),
    ));
    let mut harness = Harness::new(Resolver::new(Some("anthropic")));
    harness.interceptors.push(first.clone());
    harness.interceptors.push(second.clone());
    let seams = harness.seams();
    let mut entry = request(
        "claude-sonnet-4-20250514",
        vec![json(r#"{"role":"user","content":"hi"}"#)],
    );
    entry.max_tokens = 64;
    entry.stream = Some(true);
    entry.tools = Some(vec![json(r#"{"name":"Bash"}"#)]);
    entry.metadata = Some(json(r#"{"user_id":"u1"}"#));
    entry.stop_sequences = Some(json(r#"["stop"]"#));
    entry.system = Some(json(r#""sys""#));
    entry.temperature = Some(json("0.5"));
    entry.thinking = Some(json(r#"{"type":"enabled","budget_tokens":1024}"#));
    entry.tool_choice = Some(json(r#"{"type":"auto"}"#));
    entry.top_k = Some(json("5"));
    entry.top_p = Some(json("0.9"));
    entry.api_key = Some(SecretString::from("sk-test".to_owned()));
    entry.api_base = Some("https://api.example".to_owned());
    entry.kwargs.insert("custom".to_owned(), json("1"));
    let result = returned(execute(entry, &seams).await.unwrap());
    let MessagesEntryResult::Response(value) = result else {
        panic!("the interceptor's response must be returned");
    };
    assert_eq!(value, json(r#"{"handled":"second"}"#));
    assert_eq!(
        first
            .can_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len(),
        1_usize
    );
    assert_eq!(
        first
            .handled
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len(),
        0_usize
    );
    let handled = second
        .handled
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(handled.len(), 1_usize);
    let sent = &handled[0];
    assert_eq!(sent["model"], json(r#""claude-sonnet-4-20250514""#));
    assert_eq!(sent["max_tokens"], json("64"));
    assert_eq!(sent["stream"], Json::Bool(true));
    assert_eq!(sent["tools"], json(r#"[{"name":"Bash"}]"#));
    assert_eq!(sent["custom_llm_provider"], json(r#""anthropic""#));
    assert_eq!(sent["api_key"], json(r#""sk-test""#));
    assert_eq!(sent["api_base"], json(r#""https://api.example""#));
    assert_eq!(sent["metadata"], json(r#"{"user_id":"u1"}"#));
    assert_eq!(sent["stop_sequences"], json(r#"["stop"]"#));
    assert_eq!(sent["system"], json(r#""sys""#));
    assert_eq!(sent["temperature"], json("0.5"));
    assert_eq!(
        sent["thinking"],
        json(r#"{"type":"enabled","budget_tokens":1024}"#)
    );
    assert_eq!(sent["tool_choice"], json(r#"{"type":"auto"}"#));
    assert_eq!(sent["top_k"], json("5"));
    assert_eq!(sent["top_p"], json("0.9"));
    assert_eq!(sent["kwargs"], json(r#"{"custom":1}"#));
    assert_eq!(
        sent["messages"],
        json(r#"[{"role":"user","content":"hi"}]"#)
    );
}

#[tokio::test]
async fn interceptor_none_and_error_pass_through() {
    let quiet = Arc::new(Interceptor::new(true, InterceptorAnswer::None));
    let mut harness = Harness::new(Resolver::new(Some("anthropic")));
    harness.interceptors.push(quiet);
    let seams = harness.seams();
    let entry = request(
        "claude-sonnet-4-20250514",
        vec![json(r#"{"role":"user","content":"hi"}"#)],
    );
    let result = returned(execute(entry, &seams).await.unwrap());
    assert!(matches!(result, MessagesEntryResult::None));

    let failing = Arc::new(Interceptor::new(true, InterceptorAnswer::Raise(raised())));
    let mut raised_harness = Harness::new(Resolver::new(Some("anthropic")));
    raised_harness.interceptors.push(failing);
    let raised_seams = raised_harness.seams();
    let raised_entry = request(
        "claude-sonnet-4-20250514",
        vec![json(r#"{"role":"user","content":"hi"}"#)],
    );
    let error = failure(execute(raised_entry, &raised_seams).await);
    assert!(matches!(error, MessagesExecuteError::Raised(_)));
}

#[tokio::test]
async fn final_dispatch_preserves_context() {
    let hook = Arc::new(Hook::new(HookEffect::Replace(map(
        r#"{"system":"hooked","litellm_params":{"custom_llm_provider":"anthropic"}}"#,
    ))));
    let mut harness = Harness::new(Resolver::new(Some("anthropic")));
    harness.hooks.push(hook);
    let seams = harness.seams();
    let mut entry = request(
        "claude-sonnet-4-20250514",
        vec![json(r#"{"role":"user","content":"hi"}"#)],
    );
    entry.stream = Some(false);
    entry.metadata = Some(json(r#"{"user_id":"u1"}"#));
    entry.api_key = Some(SecretString::from("sk-test".to_owned()));
    entry.api_base = Some("https://api.example".to_owned());
    entry.system = Some(json(r#""sys""#));
    entry.kwargs.insert("custom".to_owned(), json("1"));
    let prepared = continuation(execute(entry, &seams).await.unwrap());
    let dispatch = &prepared.dispatch;
    assert_eq!(dispatch.max_tokens, 32);
    assert_eq!(dispatch.model, "claude-sonnet-4-20250514");
    assert_eq!(dispatch.stream, Some(false));
    assert_eq!(dispatch.system, Some(json(r#""hooked""#)));
    assert_eq!(dispatch.metadata, Some(json(r#"{"user_id":"u1"}"#)));
    assert_eq!(dispatch.api_base, Some("https://api.example".to_owned()));
    assert!(dispatch.api_key.is_some());
    assert!(dispatch.client.is_none());
    assert_eq!(dispatch.custom_llm_provider, Some("anthropic".to_owned()));
    assert_eq!(dispatch.kwargs["is_async"], Json::Bool(true));
    assert_eq!(dispatch.kwargs["custom"], json("1"));
    assert!(dispatch.presanitized);
    assert!(
        !dispatch
            .kwargs
            .contains_key("_litellm_messages_presanitized")
    );
    let value = json(r#"{"id":"msg_1"}"#);
    let resumed = prepared
        .resume(MessagesDispatchOutcome::Returned(
            MessagesEntryResult::Response(value.clone()),
        ))
        .unwrap();
    let MessagesEntryResult::Response(returned_value) = resumed else {
        panic!("the returned response must come back unchanged");
    };
    assert_eq!(returned_value, value);
}

#[tokio::test]
async fn advisor_without_interceptor_is_refused() {
    let harness = Harness::new(Resolver::new(Some("bedrock")));
    let seams = harness.seams();
    let mut entry = request(
        "claude-sonnet-4-20250514",
        vec![json(r#"{"role":"user","content":"hi"}"#)],
    );
    entry.tools = Some(vec![json(
        r#"{"type":"advisor_20260301","name":"advisor"}"#,
    )]);
    let error = failure(execute(entry, &seams).await);
    assert_eq!(
        error.to_string(),
        "the advisor_20260301 tool needs the advisor loop, which is not built yet"
    );
}

#[tokio::test]
async fn advisor_on_native_provider_dispatches() {
    let harness = Harness::new(Resolver::new(Some("anthropic")));
    let seams = harness.seams();
    let mut entry = request(
        "claude-sonnet-4-20250514",
        vec![json(r#"{"role":"user","content":"hi"}"#)],
    );
    entry.tools = Some(vec![json(
        r#"{"type":"advisor_20260301","name":"advisor"}"#,
    )]);
    let prepared = continuation(execute(entry, &seams).await.unwrap());
    assert_eq!(
        prepared.dispatch.custom_llm_provider,
        Some("anthropic".to_owned())
    );

    let empty_tools = request(
        "claude-sonnet-4-20250514",
        vec![json(r#"{"role":"user","content":"hi"}"#)],
    );
    let native = Harness::new(Resolver::new(Some("bedrock")));
    let native_seams = native.seams();
    let mut with_empty = empty_tools;
    with_empty.tools = Some(Vec::new());
    continuation(execute(with_empty, &native_seams).await.unwrap());
}

#[tokio::test]
async fn hook_error_stops_before_search_and_interceptors() {
    let searcher = Arc::new(Searcher::new(Some(json("{}")), None));
    let interceptor = Arc::new(Interceptor::new(true, InterceptorAnswer::None));
    let mut harness = Harness::new(Resolver::new(Some("anthropic")));
    harness
        .hooks
        .push(Arc::new(Hook::new(HookEffect::Raise(raised()))));
    harness.searchers.push(searcher.clone());
    harness.interceptors.push(interceptor.clone());
    let seams = harness.seams();
    let entry = request(
        "claude-sonnet-4-20250514",
        vec![json(r#"{"role":"user","content":"hi"}"#)],
    );
    let error = failure(execute(entry, &seams).await);
    let MessagesExecuteError::Raised(inner) = error else {
        panic!("a hook error must stay unclassified for @client");
    };
    assert_eq!(inner.class, "BadRequestError");
    assert_eq!(
        searcher
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len(),
        0_usize
    );
    assert_eq!(
        interceptor
            .can_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len(),
        0_usize
    );
}

#[tokio::test]
async fn searcher_error_stops_before_interceptors() {
    let interceptor = Arc::new(Interceptor::new(true, InterceptorAnswer::None));
    let mut harness = Harness::new(Resolver::new(Some("anthropic")));
    harness
        .searchers
        .push(Arc::new(Searcher::new(None, Some(raised()))));
    harness.interceptors.push(interceptor.clone());
    let seams = harness.seams();
    let entry = request(
        "claude-sonnet-4-20250514",
        vec![json(r#"{"role":"user","content":"hi"}"#)],
    );
    let error = failure(execute(entry, &seams).await);
    assert!(matches!(error, MessagesExecuteError::Raised(_)));
    assert_eq!(
        interceptor
            .can_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len(),
        0_usize
    );
}

#[tokio::test]
async fn dispatch_errors_keep_the_boundary() {
    let harness = Harness::new(Resolver::new(Some("anthropic")));
    let seams = harness.seams();
    let entry = request(
        "claude-sonnet-4-20250514",
        vec![json(r#"{"role":"user","content":"hi"}"#)],
    );
    let prepared = continuation(execute(entry, &seams).await.unwrap());
    let outcome = prepared.resume(MessagesDispatchOutcome::BaseLlmException(
        MessagesBaseLlmException {
            status_code: 401,
            message: r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#.to_owned(),
        },
    ));
    let error = match outcome {
        Ok(_) => panic!("a BaseLLMException must not resume as a result"),
        Err(error) => error,
    };
    let MessagesExecuteError::Classified(class) = error else {
        panic!("the dispatch boundary must classify a BaseLLMException");
    };
    assert!(matches!(class.as_ref(), ErrorClass::Authentication(_)));
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, 401);
    assert_eq!(metadata.error_type, WireField::Null);
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.openai_code, None);
    assert_eq!(metadata.provider, Some("anthropic".to_owned()));

    let untouched = prepared.resume(MessagesDispatchOutcome::Raised(raised()));
    let raised_error = match untouched {
        Ok(_) => panic!("a raised error must not resume as a result"),
        Err(error) => error,
    };
    assert!(matches!(raised_error, MessagesExecuteError::Raised(_)));
}
