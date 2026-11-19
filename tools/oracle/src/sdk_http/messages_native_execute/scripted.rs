use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::api::messages_native_execute::entry::MessagesEntryResult;
use betterlite_core::api::messages_native_execute::error::MessagesExecuteError;
use betterlite_core::api::messages_native_execute::seams::{
    CacheControlHook, CacheControlOutcome, CacheControlRequest, InterceptorRequest,
    MessagesInterceptor, PreRequestHook, SearchShortCircuit,
};
use betterlite_core::api::messages_reasoning_route::seams::{
    ProviderLookup, ProviderResolution, ResolvedTarget,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::{Call, HookScript, InterceptorScript, Outcome, SearchScript};

pub(super) type Record = IndexMap<String, Json>;

pub(super) const API_BASE: &str = "<api_base>";

pub(super) fn optional(value: Option<&Json>) -> Json {
    value.cloned().unwrap_or(Json::Null)
}

pub(super) fn optional_text(value: Option<&str>) -> Json {
    value.map_or(Json::Null, |text| Json::Text(text.to_owned()))
}

// The harness's mock base must not be compared, so its record keeps a marker instead
pub(super) fn api_base(value: Option<&str>) -> Json {
    value.map_or(Json::Null, |_found| Json::Text(API_BASE.to_owned()))
}

pub(super) fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

fn list_value(values: Option<&[Json]>) -> Json {
    values.map_or(Json::Null, |items| Json::List(items.to_vec()))
}

fn take_calls(store: &Mutex<Vec<Record>>) -> Vec<Record> {
    store.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

#[derive(Default)]
pub(super) struct ScriptedCache {
    first: Mutex<Option<Record>>,
}

impl ScriptedCache {
    pub(super) fn take(&self) -> Record {
        let slot = self.first.lock().unwrap_or_else(PoisonError::into_inner);
        slot.clone().unwrap_or_default()
    }
}

impl CacheControlHook for ScriptedCache {
    fn maybe_inject_cache_control(
        &self,
        request: CacheControlRequest<'_>,
    ) -> Result<CacheControlOutcome, MessagesExecuteError> {
        let mut record: Record = IndexMap::new();
        record.insert("messages".to_owned(), Json::List(request.messages.to_vec()));
        record.insert("system".to_owned(), optional(request.system));
        record.insert("model".to_owned(), Json::Text(request.model.to_owned()));
        record.insert(
            "custom_llm_provider".to_owned(),
            optional_text(request.custom_llm_provider),
        );
        record.insert("tools".to_owned(), list_value(request.tools));
        record.insert("api_base".to_owned(), api_base(request.api_base));
        record.insert("kwargs".to_owned(), Json::Object(request.kwargs.clone()));
        let mut slot = self.first.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.is_none() {
            *slot = Some(record);
        }
        drop(slot);
        Ok(CacheControlOutcome {
            messages: request.messages.to_vec(),
            system: request.system.cloned(),
        })
    }
}

pub(super) struct ScriptedHook {
    script: HookScript,
    calls: Mutex<Vec<Record>>,
}

impl ScriptedHook {
    pub(super) fn new(script: HookScript) -> Self {
        Self {
            script,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn take(&self) -> Vec<Record> {
        take_calls(&self.calls)
    }
}

#[async_trait]
impl PreRequestHook for ScriptedHook {
    fn callback_name(&self) -> &str {
        "scripted"
    }

    async fn pre_request(
        &self,
        model: &str,
        messages: &[Json],
        kwargs: &mut IndexMap<String, Json>,
    ) -> Result<Option<IndexMap<String, Json>>, MessagesExecuteError> {
        let mut record: Record = IndexMap::new();
        record.insert("model".to_owned(), Json::Text(model.to_owned()));
        record.insert("messages".to_owned(), Json::List(messages.to_vec()));
        record.insert("kwargs".to_owned(), Json::Object(kwargs.clone()));
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.push(record);
        drop(calls);
        match &self.script {
            HookScript::Pass => Ok(None),
            HookScript::Mutate { values } => {
                for (key, value) in values {
                    kwargs.insert(key.clone(), value.clone());
                }
                Ok(None)
            }
            HookScript::Replace { values } => Ok(Some(values.clone())),
        }
    }
}

pub(super) struct ScriptedSearch {
    answer: Option<Json>,
}

#[async_trait]
impl SearchShortCircuit for ScriptedSearch {
    fn callback_name(&self) -> &str {
        "scripted-search"
    }

    async fn try_short_circuit_search(
        &self,
        _model: &str,
        _messages: &[Json],
        _tools: Option<&[Json]>,
        _provider: Option<&str>,
        _kwargs: &IndexMap<String, Json>,
    ) -> Result<Option<Json>, MessagesExecuteError> {
        Ok(self.answer.clone())
    }
}

pub(super) struct ScriptedInterceptor {
    script: InterceptorScript,
}

#[async_trait]
impl MessagesInterceptor for ScriptedInterceptor {
    fn can_handle(&self, _tools: Option<&[Json]>, _provider: Option<&str>) -> bool {
        self.script.can_handle
    }

    async fn handle(
        &self,
        _request: &InterceptorRequest<'_>,
    ) -> Result<MessagesEntryResult, MessagesExecuteError> {
        match &self.script.answer {
            Json::Null => Ok(MessagesEntryResult::None),
            value => Ok(MessagesEntryResult::Response(value.clone())),
        }
    }
}

pub(super) struct ScriptedResolver {
    provider: Option<String>,
}

impl ProviderResolution for ScriptedResolver {
    fn resolve(&self, lookup: &ProviderLookup<'_>) -> Option<ResolvedTarget> {
        let provider = self.provider.clone()?;
        Some(ResolvedTarget {
            model: lookup.model.unwrap_or_default().to_owned(),
            provider,
            api_base: None,
        })
    }
}

// The scripted seams in the case's order; the entry borrows the trait objects, the records stay here
pub(super) struct Scripts {
    pub(super) cache: ScriptedCache,
    resolver: ScriptedResolver,
    hooks: Vec<Arc<ScriptedHook>>,
    pub(super) hook_objects: Vec<Arc<dyn PreRequestHook>>,
    pub(super) search_objects: Vec<Arc<dyn SearchShortCircuit>>,
    pub(super) interceptor_objects: Vec<Arc<dyn MessagesInterceptor>>,
}

impl Scripts {
    pub(super) fn new(call: &Call) -> Self {
        let hooks: Vec<Arc<ScriptedHook>> = call
            .hooks
            .iter()
            .cloned()
            .map(|script| Arc::new(ScriptedHook::new(script)))
            .collect();
        let searches: Vec<Arc<ScriptedSearch>> = call
            .searches
            .iter()
            .cloned()
            .map(|script| {
                Arc::new(ScriptedSearch {
                    answer: script.answer,
                })
            })
            .collect();
        let interceptors: Vec<Arc<ScriptedInterceptor>> = call
            .interceptors
            .iter()
            .cloned()
            .map(|script| Arc::new(ScriptedInterceptor { script }))
            .collect();
        let mut hook_objects: Vec<Arc<dyn PreRequestHook>> = Vec::with_capacity(hooks.len());
        for hook in &hooks {
            let object: Arc<dyn PreRequestHook> = Arc::clone(hook);
            hook_objects.push(object);
        }
        let mut search_objects: Vec<Arc<dyn SearchShortCircuit>> =
            Vec::with_capacity(searches.len());
        for search in &searches {
            let object: Arc<dyn SearchShortCircuit> = Arc::clone(search);
            search_objects.push(object);
        }
        let mut interceptor_objects: Vec<Arc<dyn MessagesInterceptor>> =
            Vec::with_capacity(interceptors.len());
        for interceptor in &interceptors {
            let object: Arc<dyn MessagesInterceptor> = Arc::clone(interceptor);
            interceptor_objects.push(object);
        }
        Self {
            cache: ScriptedCache::default(),
            resolver: ScriptedResolver {
                provider: call.provider.clone(),
            },
            hooks,
            hook_objects,
            search_objects,
            interceptor_objects,
        }
    }

    pub(super) fn resolver(&self) -> &ScriptedResolver {
        &self.resolver
    }

    pub(super) fn fill(&self, outcome: &mut Outcome) {
        outcome.cache = self.cache.take();
        outcome.hooks = self.take_hooks();
    }

    pub(super) fn take_hooks(&self) -> Vec<Record> {
        let mut found = Vec::new();
        for hook in &self.hooks {
            found.extend(hook.take());
        }
        found
    }
}
