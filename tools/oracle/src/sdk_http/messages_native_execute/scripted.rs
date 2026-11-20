use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::api::messages_native_execute::entry::MessagesEntryResult;
use betterlite_core::api::messages_native_execute::error::MessagesExecuteError;
use betterlite_core::api::messages_native_execute::seams::{
    CacheControlHook, CacheControlOutcome, CacheControlRequest, InterceptorRequest,
    MessagesInterceptor, PreRequestHook, SearchRequest, SearchShortCircuit,
};
use betterlite_core::api::messages_reasoning_route::seams::{
    ProviderLookup, ProviderResolution, ResolvedTarget,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::{Call, HookScript, InterceptorScript};

pub(super) type Record = IndexMap<String, Json>;

pub(super) const API_BASE: &str = "<api_base>";

// The cache hook and every pre-hook write here, so one lock holds all the records the case observed
#[derive(Default)]
pub(super) struct Recordings {
    pub(super) cache: Option<Record>,
    pub(super) hooks: Vec<Record>,
}

pub(super) struct ScriptedCache {
    pub(super) recordings: Arc<Mutex<Recordings>>,
}

fn cache_record(request: &CacheControlRequest<'_>) -> Record {
    let mut record: Record = IndexMap::new();
    record.insert("messages".to_owned(), Json::List(request.messages.to_vec()));
    record.insert(
        "system".to_owned(),
        request.system.cloned().unwrap_or(Json::Null),
    );
    record.insert("model".to_owned(), Json::Text(request.model.to_owned()));
    record.insert(
        "custom_llm_provider".to_owned(),
        request
            .custom_llm_provider
            .map_or(Json::Null, |text| Json::Text(text.to_owned())),
    );
    record.insert(
        "tools".to_owned(),
        request
            .tools
            .map_or(Json::Null, |items| Json::List(items.to_vec())),
    );
    // the harness's mock base must not be compared, so its record keeps a marker instead
    record.insert(
        "api_base".to_owned(),
        request
            .api_base
            .map_or(Json::Null, |_found| Json::Text(API_BASE.to_owned())),
    );
    record.insert("kwargs".to_owned(), Json::Object(request.kwargs.clone()));
    record
}

impl CacheControlHook for ScriptedCache {
    fn maybe_inject_cache_control(
        &self,
        request: CacheControlRequest<'_>,
    ) -> Result<CacheControlOutcome, MessagesExecuteError> {
        let record = cache_record(&request);
        let mut recordings = self
            .recordings
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if recordings.cache.is_none() {
            recordings.cache = Some(record);
        }
        drop(recordings);
        Ok(CacheControlOutcome {
            messages: request.messages.to_vec(),
            system: request.system.cloned(),
        })
    }
}

pub(super) struct ScriptedHook {
    pub(super) script: HookScript,
    pub(super) recordings: Arc<Mutex<Recordings>>,
}

#[async_trait]
impl PreRequestHook for ScriptedHook {
    fn callback_name(&self) -> &'static str {
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
        self.recordings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .hooks
            .push(record);
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
    fn callback_name(&self) -> &'static str {
        "scripted-search"
    }

    async fn try_short_circuit_search(
        &self,
        _request: &SearchRequest<'_>,
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

// The case's scripted seams in order; the entry borrows the trait objects, the records stay here
pub(super) struct Scripts {
    pub(super) cache: ScriptedCache,
    pub(super) resolver: ScriptedResolver,
    pub(super) recordings: Arc<Mutex<Recordings>>,
    pub(super) hooks: Vec<Arc<dyn PreRequestHook>>,
    pub(super) searches: Vec<Arc<dyn SearchShortCircuit>>,
    pub(super) interceptors: Vec<Arc<dyn MessagesInterceptor>>,
}

impl Scripts {
    pub(super) fn new(call: &Call) -> Self {
        let recordings = Arc::new(Mutex::new(Recordings::default()));
        let mut hooks: Vec<Arc<dyn PreRequestHook>> = Vec::new();
        for script in &call.hooks {
            hooks.push(Arc::new(ScriptedHook {
                script: script.clone(),
                recordings: Arc::clone(&recordings),
            }));
        }
        let mut searches: Vec<Arc<dyn SearchShortCircuit>> = Vec::new();
        for search in &call.searches {
            searches.push(Arc::new(ScriptedSearch {
                answer: search.answer.clone(),
            }));
        }
        let mut interceptors: Vec<Arc<dyn MessagesInterceptor>> = Vec::new();
        for interceptor in &call.interceptors {
            interceptors.push(Arc::new(ScriptedInterceptor {
                script: interceptor.clone(),
            }));
        }
        Self {
            cache: ScriptedCache {
                recordings: Arc::clone(&recordings),
            },
            resolver: ScriptedResolver {
                provider: call.provider.clone(),
            },
            recordings,
            hooks,
            searches,
            interceptors,
        }
    }
}
