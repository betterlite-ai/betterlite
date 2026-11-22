use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::resolver::LoggerResolver;
use betterlite_logging::registry::error::RegistryError;
use betterlite_openai::openai_base_config::Object as JsonMap;
use http::request::Parts;

use crate::pre_call::failure::PreCallFailure;
use crate::pre_call::key::PreCallKey;
use crate::pre_call::ports::{
    CompressionArming, GuardrailMerge, PreCallHooks, RequestPreparer, RetrievalPipelines,
    RouterSettingsSource, StreamOptionsCheck, TagBudgets, VectorStoreAccess,
};
use crate::pre_call::rewrites::RequestModel;
use crate::pre_call::route_type::RouteType;

pub(super) struct PassthroughPreparer;

#[async_trait]
impl<K: PreCallKey> RequestPreparer<K> for PassthroughPreparer {
    async fn prepare(
        &self,
        data: JsonMap,
        _request: &Parts,
        _key: &K,
        _version: Option<&str>,
    ) -> Result<JsonMap, PreCallFailure> {
        Ok(data)
    }
}

pub(super) struct AllowAllStores;

#[async_trait]
impl<K: PreCallKey> VectorStoreAccess<K> for AllowAllStores {
    async fn assert_access(&self, _store_id: &str, _key: &K) -> Result<(), PreCallFailure> {
        Ok(())
    }
}

pub(super) struct NoRouterSettings;

#[async_trait]
impl<K: PreCallKey> RouterSettingsSource<K> for NoRouterSettings {
    async fn router_settings(&self, _key: &K) -> Result<Option<JsonMap>, PreCallFailure> {
        Ok(None)
    }
}

pub(super) struct NeverSupports;

impl StreamOptionsCheck for NeverSupports {
    fn supports(&self, _litellm_model: &str) -> bool {
        false
    }
}

pub(super) struct PassthroughMerge;

#[async_trait]
impl GuardrailMerge for PassthroughMerge {
    async fn merge(&self, data: JsonMap, _model_alias: Option<&str>) -> JsonMap {
        data
    }
}

pub(super) struct NoCompression;

#[async_trait]
impl CompressionArming for NoCompression {
    async fn arm(&self, _data: &mut JsonMap) {}
}

pub(super) struct PassthroughHooks;

#[async_trait]
impl<K: PreCallKey> PreCallHooks<K> for PassthroughHooks {
    async fn pre_call_hook(
        &self,
        _key: &K,
        data: JsonMap,
        _route_type: RouteType,
        _skip_guardrails: bool,
    ) -> Result<JsonMap, PreCallFailure> {
        Ok(data)
    }
}

pub(super) struct AllowAllBudgets;

#[async_trait]
impl<K: PreCallKey> TagBudgets<K> for AllowAllBudgets {
    async fn skips_budget(&self, _route: &str, _model: &RequestModel) -> bool {
        true
    }

    async fn check_tags(&self, _tags: &[String], _key: &K) -> Result<(), PreCallFailure> {
        Ok(())
    }
}

pub(super) struct NoRetrieval;

#[async_trait]
impl<K: PreCallKey> RetrievalPipelines<K> for NoRetrieval {
    async fn attach(&self, _data: &mut JsonMap, _key: &K) {}
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
