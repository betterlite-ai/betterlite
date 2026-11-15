#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use std::str::FromStr;

use crate::auth_request_models::catalog::{
    ManagedIdDecoders, ManagedIdError, ManagedIdFamily, ModelCatalog,
};
use crate::reservation_cost::error::{ModelInfoError, ReservationCostError};
use crate::reservation_cost::{
    EstimateRequest, EstimateSources, ModelInfoSource, ReservationTokens, RouterDeployment,
    RouterModelInfo, count_request_input_tokens, estimate_request_input_cost,
    estimate_request_max_cost,
};

pub(crate) fn json(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("the fixture is valid JSON")
}

pub(crate) fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).expect("the expected amount is a decimal")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Failing {
    Group,
    Deployments,
    DeploymentInfo,
}

#[derive(Debug, Default)]
pub(crate) struct FixedRouter {
    pub(crate) groups: BTreeMap<String, JsonObject>,
    pub(crate) deployments: BTreeMap<String, Vec<RouterDeployment>>,
    pub(crate) deployment_infos: BTreeMap<String, JsonObject>,
    pub(crate) failing: Option<Failing>,
}

impl RouterModelInfo for FixedRouter {
    fn get_model_list(&self, model_name: &str) -> Result<Vec<RouterDeployment>, ModelInfoError> {
        if self.failing == Some(Failing::Deployments) {
            return Err(ModelInfoError::Unavailable);
        }
        Ok(self
            .deployments
            .get(model_name)
            .cloned()
            .unwrap_or_default())
    }

    fn cached_deployment_model_info(
        &self,
        model_id: &str,
        _backend_model: &str,
    ) -> Result<Option<JsonObject>, ModelInfoError> {
        if self.failing == Some(Failing::DeploymentInfo) {
            return Err(ModelInfoError::Unavailable);
        }
        Ok(self.deployment_infos.get(model_id).cloned())
    }

    fn cached_model_group_info(
        &self,
        model_name: &str,
    ) -> Result<Option<JsonObject>, ModelInfoError> {
        if self.failing == Some(Failing::Group) {
            return Err(ModelInfoError::Unavailable);
        }
        Ok(self.groups.get(model_name).cloned())
    }
}

#[derive(Debug, Default)]
pub(crate) struct FixedModels {
    pub(crate) infos: BTreeMap<String, JsonObject>,
    pub(crate) failure: bool,
}

impl ModelInfoSource for FixedModels {
    fn get_model_info(&self, model: &str) -> Result<Option<JsonObject>, ModelInfoError> {
        if self.failure {
            return Err(ModelInfoError::Unavailable);
        }
        Ok(self.infos.get(model).cloned())
    }
}

#[derive(Debug, Default)]
pub(crate) struct RecordingTokens {
    pub(crate) counts: BTreeMap<String, usize>,
    counted: Mutex<Vec<String>>,
    map_calls: AtomicUsize,
}

impl RecordingTokens {
    pub(crate) fn counted_models(&self) -> Vec<String> {
        self.counted
            .lock()
            .expect("the recorder is not poisoned")
            .clone()
    }

    pub(crate) fn map_calls(&self) -> usize {
        self.map_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ReservationTokens for RecordingTokens {
    async fn count_input_tokens(
        &self,
        _body: &JsonObject,
        models: &[String],
    ) -> IndexMap<String, usize> {
        self.map_calls.fetch_add(1, Ordering::SeqCst);
        let mut counts = IndexMap::new();
        for model in models {
            if let Some(count) = self.counts.get(model) {
                counts.insert(model.clone(), *count);
            }
        }
        counts
    }

    async fn count_input_tokens_for_model(&self, _body: &JsonObject, model: &str) -> Option<usize> {
        self.counted
            .lock()
            .expect("the recorder is not poisoned")
            .push(model.to_owned());
        self.counts.get(model).copied()
    }
}

#[derive(Debug, Default)]
pub(crate) struct NoDecoders;

impl ManagedIdDecoders for NoDecoders {
    fn decode_family(
        &self,
        _family: ManagedIdFamily,
        _resource_id: &str,
        _catalog: Option<&dyn ModelCatalog>,
        _candidates: &mut Vec<String>,
    ) -> Result<(), ManagedIdError> {
        Ok(())
    }
}

pub(crate) struct Fixture {
    body: JsonObject,
    route: String,
    router: Option<FixedRouter>,
    models: FixedModels,
    tokens: RecordingTokens,
    decoders: NoDecoders,
    provided: Option<IndexMap<String, usize>>,
}

impl Fixture {
    pub(crate) fn new(body: &str) -> Self {
        Self {
            body: json(body),
            route: "/v1/chat/completions".to_owned(),
            router: None,
            models: FixedModels::default(),
            tokens: RecordingTokens::default(),
            decoders: NoDecoders,
            provided: None,
        }
    }

    #[must_use]
    pub(crate) fn route(mut self, route: &str) -> Self {
        self.route = route.to_owned();
        self
    }

    #[must_use]
    pub(crate) fn router(mut self, router: FixedRouter) -> Self {
        self.router = Some(router);
        self
    }

    #[must_use]
    pub(crate) fn model_info(mut self, model: &str, info: &str) -> Self {
        self.models.infos.insert(model.to_owned(), json(info));
        self
    }

    #[must_use]
    pub(crate) fn model_info_failure(mut self) -> Self {
        self.models.failure = true;
        self
    }

    #[must_use]
    pub(crate) fn token_count(mut self, model: &str, count: usize) -> Self {
        self.tokens.counts.insert(model.to_owned(), count);
        self
    }

    #[must_use]
    pub(crate) fn provided_count(mut self, model: &str, count: usize) -> Self {
        let mut provided = self.provided.take().unwrap_or_default();
        provided.insert(model.to_owned(), count);
        self.provided = Some(provided);
        self
    }

    pub(crate) async fn max_cost(&self) -> Result<Option<Decimal>, ReservationCostError> {
        let request = EstimateRequest {
            body: &self.body,
            route: &self.route,
        };
        estimate_request_max_cost(&request, &self.sources(), self.provided.as_ref()).await
    }

    pub(crate) async fn input_cost(&self) -> Result<Option<Decimal>, ReservationCostError> {
        let request = EstimateRequest {
            body: &self.body,
            route: &self.route,
        };
        estimate_request_input_cost(&request, &self.sources(), self.provided.as_ref()).await
    }

    pub(crate) async fn counted(&self) -> Result<IndexMap<String, usize>, ReservationCostError> {
        let request = EstimateRequest {
            body: &self.body,
            route: &self.route,
        };
        count_request_input_tokens(&request, &self.sources()).await
    }

    pub(crate) fn tokens(&self) -> &RecordingTokens {
        &self.tokens
    }

    fn sources(&self) -> EstimateSources<'_> {
        EstimateSources {
            router: self
                .router
                .as_ref()
                .map(|router| -> &dyn RouterModelInfo { router }),
            model_info: &self.models,
            tokens: &self.tokens,
            catalog: None,
            decoders: &self.decoders,
        }
    }
}

pub(crate) fn deployment(model_id: Option<&str>, backend_model: Option<&str>) -> RouterDeployment {
    RouterDeployment {
        model_id: model_id.map(str::to_owned),
        backend_model: backend_model.map(str::to_owned),
    }
}
