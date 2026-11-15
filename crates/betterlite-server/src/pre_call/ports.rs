use std::sync::Arc;

use async_trait::async_trait;
use betterlite_logging::datadog::apm::Tracer;
use http::request::Parts;
use serde_json::{Map, Value};

use crate::pre_call::failure::PreCallFailure;
use crate::pre_call::key::PreCallKey;
use crate::pre_call::rewrites::RequestModel;
use crate::pre_call::route_type::RouteType;

#[async_trait]
pub trait RequestPreparer<K: PreCallKey>: Send + Sync {
    async fn prepare(
        &self,
        data: Map<String, Value>,
        request: &Parts,
        key: &K,
        version: Option<&str>,
    ) -> Result<Map<String, Value>, PreCallFailure>;
}

#[async_trait]
pub trait VectorStoreAccess<K: PreCallKey>: Send + Sync {
    async fn assert_access(&self, store_id: &str, key: &K) -> Result<(), PreCallFailure>;
}

#[async_trait]
pub trait RouterSettingsSource<K: PreCallKey>: Send + Sync {
    async fn router_settings(&self, key: &K) -> Result<Option<Map<String, Value>>, PreCallFailure>;
}

#[async_trait]
pub trait RouterView<K: PreCallKey>: Send + Sync {
    // Router.resolved_litellm_models is a plain in-memory read, and the stream_options check calls it lazily from a sync closure
    fn resolved_models(&self, model: &str, team_id: Option<&str>) -> Vec<String>;

    fn arm_prefetch(&self, model: &str, data: &Map<String, Value>);

    async fn fallbacks(&self) -> Option<Vec<Value>>;

    async fn fallback_models(&self, model: &str, fallbacks: &[Value]) -> Option<Vec<String>>;

    async fn authorize_alias(&self, target: &str, key: &K) -> Result<(), PreCallFailure>;
}

// get_supported_openai_params is a cached in-memory lookup, so the check stays sync and the closure in stream_usage_updates can call it lazily
pub trait StreamOptionsCheck: Send + Sync {
    fn supports(&self, litellm_model: &str) -> bool;
}

#[async_trait]
pub trait GuardrailMerge: Send + Sync {
    async fn merge(
        &self,
        data: Map<String, Value>,
        model_alias: Option<&str>,
    ) -> Map<String, Value>;
}

#[async_trait]
pub trait CompressionArming: Send + Sync {
    async fn arm(&self, data: &mut Map<String, Value>);
}

#[async_trait]
pub trait PreCallHooks<K: PreCallKey>: Send + Sync {
    async fn pre_call_hook(
        &self,
        key: &K,
        data: Map<String, Value>,
        route_type: RouteType,
        skip_guardrails: bool,
    ) -> Result<Map<String, Value>, PreCallFailure>;
}

#[async_trait]
pub trait TagBudgets<K: PreCallKey>: Send + Sync {
    async fn skips_budget(&self, route: &str, model: &RequestModel) -> bool;

    async fn check_tags(&self, tags: &[String], key: &K) -> Result<(), PreCallFailure>;
}

#[async_trait]
pub trait RetrievalPipelines<K: PreCallKey>: Send + Sync {
    async fn attach(&self, data: &mut Map<String, Value>, key: &K);
}

pub struct PreCallPorts<K: PreCallKey> {
    pub preparer: Arc<dyn RequestPreparer<K>>,
    pub vector_stores: Arc<dyn VectorStoreAccess<K>>,
    pub router_settings: Arc<dyn RouterSettingsSource<K>>,
    pub router: Option<Arc<dyn RouterView<K>>>,
    pub stream_options: Arc<dyn StreamOptionsCheck>,
    pub guardrail_merge: Arc<dyn GuardrailMerge>,
    pub compression: Arc<dyn CompressionArming>,
    pub hooks: Arc<dyn PreCallHooks<K>>,
    pub tag_budgets: Arc<dyn TagBudgets<K>>,
    pub retrieval: Arc<dyn RetrievalPipelines<K>>,
    pub tracer: Arc<dyn Tracer + Send + Sync>,
}
