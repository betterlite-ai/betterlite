#![cfg(test)]

use std::future::{Future, ready};

use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::{
    CapabilityPredicates, CapabilityRequest, NamedCapability,
};
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::settings::ROISettings;

pub(super) struct ModelSupport {
    pub supported: Vec<String>,
    pub seen: Vec<(String, String)>,
}

impl CapabilityPredicates for ModelSupport {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        self.seen.push((key.to_owned(), request.model.to_owned()));
        let supported = key == NamedCapability::NoneReasoningEffort.key()
            && self.supported.iter().any(|model| model == request.model);
        ready(supported)
    }

    fn declared_value_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn supports_native_streaming(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = Json> + Send {
        ready(Json::Null)
    }

    fn supports_response_schema(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }
}

pub(super) fn settings(model: &str, prompt: &str) -> ROISettings {
    ROISettings {
        github_api_url: "https://api.github.com".to_owned(),
        github_token: SecretString::default(),
        estimator_key: SecretString::default(),
        repos: Vec::new(),
        estimator_model: model.to_owned(),
        estimator_prompt: prompt.to_owned(),
        backfill_days: 7,
        update_interval_minutes: 1440.0_f64,
        identity_map: IndexMap::new(),
    }
}
