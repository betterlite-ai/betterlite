use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::resolver::LoggerResolver;
use betterlite_logging::datadog::apm::{Span, Tracer};
use betterlite_logging::registry::error::RegistryError;
use betterlite_openai::openai_base_config::Object as JsonMap;
use http::request::Parts;

use super::{Behavior, Call, TestKey};
use crate::pre_call::failure::{PreCallFailure, ProxyFailure};
use crate::pre_call::ports::{
    CompressionArming, GuardrailMerge, PreCallHooks, RequestPreparer, RetrievalPipelines,
    RouterSettingsSource, RouterView, StreamOptionsCheck, TagBudgets, VectorStoreAccess,
};
use crate::pre_call::rewrites::RequestModel;
use crate::pre_call::route_type::RouteType;

pub(super) struct Fake {
    calls: Mutex<Vec<Call>>,
    script: Behavior,
}

impl Fake {
    pub(super) fn new(script: Behavior) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            script,
        }
    }

    fn record(&self, call: Call) {
        self.calls.lock().unwrap().push(call);
    }

    pub(super) fn events(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl RequestPreparer<TestKey> for Fake {
    async fn prepare(
        &self,
        mut data: JsonMap,
        _request: &Parts,
        _key: &TestKey,
        _version: Option<&str>,
    ) -> Result<JsonMap, PreCallFailure> {
        self.record(Call::Prepare);
        if let Some(failure) = self.script.prepare.clone() {
            return Err(failure);
        }
        if let Some(prefix) = &self.script.tag_on_prepare {
            let model = data
                .get("model")
                .and_then(JsonValue::as_str)
                .unwrap_or_default();
            let tag = format!("{prefix}-{model}");
            data.insert(
                String::from("tags"),
                JsonValue::Array(vec![JsonValue::String(tag)]),
            );
        }
        Ok(data)
    }
}

#[async_trait]
impl VectorStoreAccess<TestKey> for Fake {
    async fn assert_access(&self, store_id: &str, _key: &TestKey) -> Result<(), PreCallFailure> {
        self.record(Call::Access(store_id.to_owned()));
        match self.script.access.clone() {
            Some(failure) => Err(failure),
            None => Ok(()),
        }
    }
}

#[async_trait]
impl RouterSettingsSource<TestKey> for Fake {
    async fn router_settings(&self, _key: &TestKey) -> Result<Option<JsonMap>, PreCallFailure> {
        self.record(Call::Settings);
        Ok(self.script.settings.clone())
    }
}

#[async_trait]
impl RouterView<TestKey> for Fake {
    fn resolved_models(&self, _model: &str, _team_id: Option<&str>) -> Vec<String> {
        Vec::new()
    }

    fn arm_prefetch(&self, model: &str, _data: &JsonMap) {
        self.record(Call::Prefetch(model.to_owned()));
    }

    async fn fallbacks(&self) -> Option<Vec<JsonValue>> {
        self.record(Call::Fallbacks);
        self.script.fallbacks.clone()
    }

    async fn fallback_models(&self, model: &str, fallbacks: &[JsonValue]) -> Option<Vec<String>> {
        let listed = fallbacks
            .iter()
            .filter_map(JsonValue::as_str)
            .map(str::to_owned)
            .collect();
        self.record(Call::FallbackModels(model.to_owned(), listed));
        self.script.fallback_models.clone()
    }

    async fn authorize_alias(&self, target: &str, _key: &TestKey) -> Result<(), PreCallFailure> {
        self.record(Call::Authorize(target.to_owned()));
        match self.script.authorize.clone() {
            Some(failure) => Err(failure),
            None => Ok(()),
        }
    }
}

impl StreamOptionsCheck for Fake {
    fn supports(&self, _litellm_model: &str) -> bool {
        self.script.supports
    }
}

#[async_trait]
impl GuardrailMerge for Fake {
    async fn merge(&self, data: JsonMap, model_alias: Option<&str>) -> JsonMap {
        self.record(Call::Merge(model_alias.map(str::to_owned)));
        data
    }
}

#[async_trait]
impl CompressionArming for Fake {
    async fn arm(&self, _data: &mut JsonMap) {
        self.record(Call::Compress);
    }
}

#[async_trait]
impl PreCallHooks<TestKey> for Fake {
    async fn pre_call_hook(
        &self,
        _key: &TestKey,
        mut data: JsonMap,
        _route_type: RouteType,
        skip_guardrails: bool,
    ) -> Result<JsonMap, PreCallFailure> {
        self.record(Call::Hook(skip_guardrails));
        if let Some(failure) = self.script.hook_failure.clone() {
            return Err(failure);
        }
        if let Some(JsonValue::String(model)) = data.get("model")
            && self.script.rate_limited_models.contains(model)
        {
            return Err(rate_limited());
        }
        let tags = self.script.hook_tags.clone();
        if !tags.is_empty() {
            data.insert(
                String::from("tags"),
                JsonValue::Array(tags.into_iter().map(JsonValue::String).collect()),
            );
        }
        Ok(data)
    }
}

#[async_trait]
impl TagBudgets<TestKey> for Fake {
    async fn skips_budget(&self, route: &str, _model: &RequestModel) -> bool {
        self.record(Call::Skip(route.to_owned()));
        self.script.skip_budget
    }

    async fn check_tags(&self, tags: &[String], _key: &TestKey) -> Result<(), PreCallFailure> {
        self.record(Call::Tags(tags.to_vec()));
        Ok(())
    }
}

#[async_trait]
impl RetrievalPipelines<TestKey> for Fake {
    async fn attach(&self, _data: &mut JsonMap, _key: &TestKey) {
        self.record(Call::Attach);
    }
}

#[derive(Default)]
pub(super) struct TestTracer {
    tags: Mutex<Vec<(String, String)>>,
}

impl TestTracer {
    pub(super) fn tags(&self) -> Vec<(String, String)> {
        self.tags.lock().unwrap().clone()
    }
}

impl Span for TestTracer {
    fn set_tag(&self, key: &str, value: &str) -> bool {
        self.tags
            .lock()
            .unwrap()
            .push((key.to_owned(), value.to_owned()));
        true
    }
}

impl Tracer for TestTracer {
    fn current_span(&self) -> Option<&dyn Span> {
        Some(self)
    }
}

pub(super) struct NoLoggers;

impl LoggerResolver for NoLoggers {
    fn resolve(
        &self,
        _name: &str,
        _params: &BTreeMap<String, String>,
    ) -> Result<Option<Arc<dyn CallbackLogger>>, RegistryError> {
        Ok(None)
    }
}

pub(super) struct TestLogger;

impl CallbackLogger for TestLogger {
    fn callback_name(&self) -> &str {
        "test-logger"
    }
}

pub(super) struct TestLoggers;

impl LoggerResolver for TestLoggers {
    fn resolve(
        &self,
        name: &str,
        _params: &BTreeMap<String, String>,
    ) -> Result<Option<Arc<dyn CallbackLogger>>, RegistryError> {
        if name == "test-logger" {
            return Ok(Some(Arc::new(TestLogger)));
        }
        Ok(None)
    }
}

pub(super) fn rate_limited() -> PreCallFailure {
    PreCallFailure::RateLimited(ProxyFailure {
        message: String::from("rate limited"),
        error_type: String::from("rate_limit_error"),
        param: None,
        code: 429,
    })
}
