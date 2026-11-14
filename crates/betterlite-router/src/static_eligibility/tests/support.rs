use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_predicates::params::{ParamSupport, ResolvedProvider};
use crate::static_eligibility::optional::{CooldownRecorder, DeploymentPreCallCheck, PreCallError};
use crate::static_eligibility::request::RequestFacts;
use crate::static_eligibility::tokens::{TokenCountError, TokenCounter, TokenLimits};

pub(crate) fn facts() -> RequestFacts {
    RequestFacts {
        messages: None,
        system: None,
        tools: Vec::new(),
        response_format: false,
        target_order: None,
        input_token_count: None,
        compaction_pending: false,
        drop_params: false,
    }
}

pub(crate) fn spec(name: &str, id: &str, model: &str, info: &str) -> DeploymentSpec {
    let params = format!(r#"{{"model":"{model}"}}"#);
    let mut model_info: JsonObject = if info.is_empty() {
        JsonObject::new()
    } else {
        match serde_json::from_str(info) {
            Ok(parsed) => parsed,
            Err(error) => panic!("the test model_info does not read as JSON: {error}"),
        }
    };
    model_info.insert("id".to_owned(), Json::Text(id.to_owned()));
    DeploymentSpec::new(
        name.to_owned(),
        serde_json::from_str(&params).unwrap_or_default(),
        model_info,
        JsonObject::new(),
        String::new,
    )
    .unwrap_or_else(|error| panic!("the test deployment is invalid: {error}"))
}

pub(crate) fn with_param(spec: DeploymentSpec, key: &str, value: Json) -> DeploymentSpec {
    let mut deployment = spec;
    deployment.litellm_params.insert(key.to_owned(), value);
    deployment
}

pub(crate) fn ids(deployments: &[DeploymentSpec]) -> Vec<String> {
    deployments
        .iter()
        .map(|deployment| deployment.id().unwrap_or_default().to_owned())
        .collect()
}

// The probe's rule: the deployment's own model_info carry the window the cases exercise.
pub(crate) struct InfoLimits;

impl TokenLimits for InfoLimits {
    fn max_input_tokens(&self, spec: &DeploymentSpec, _model: &str) -> Option<usize> {
        match spec.model_info.get("max_input_tokens") {
            Some(Json::Number(number)) => number
                .as_u64()
                .and_then(|value| usize::try_from(value).ok()),
            Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
            | None => None,
        }
    }
}

pub(crate) struct Support {
    pub(crate) resolves: bool,
    pub(crate) supported: Option<Vec<String>>,
}

impl ParamSupport for Support {
    fn is_authenticating_provider(&self, _model: &str, _custom_llm_provider: Option<&str>) -> bool {
        false
    }

    fn resolve_provider(
        &self,
        model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<ResolvedProvider> {
        self.resolves.then(|| ResolvedProvider {
            model: model.to_owned(),
            custom_llm_provider: "openai".to_owned(),
        })
    }

    fn supported_openai_params(
        &self,
        _model: &str,
        _custom_llm_provider: &str,
        _base_model: Option<&str>,
    ) -> Option<Vec<String>> {
        self.supported.clone()
    }
}

pub(crate) struct FixedCounter {
    pub(crate) count: usize,
}

#[async_trait]
impl TokenCounter for FixedCounter {
    async fn count_messages(
        &self,
        messages: &[betterlite_tokenizer::count::request::Message],
        _tools: &[Json],
    ) -> Result<usize, TokenCountError> {
        Ok(self.count.saturating_add(messages.len()))
    }
}

pub(crate) struct Check {
    pub(crate) outcome: Result<(), PreCallError>,
}

#[async_trait]
impl DeploymentPreCallCheck for Check {
    fn check(&self, _deployment: &DeploymentSpec) -> Result<(), PreCallError> {
        self.outcome.clone()
    }

    async fn async_check(&self, _deployment: &DeploymentSpec) -> Result<(), PreCallError> {
        self.outcome.clone()
    }
}

#[derive(Default)]
pub(crate) struct Cooldown {
    pub(crate) recorded: Mutex<Vec<(String, u16)>>,
}

impl CooldownRecorder for Cooldown {
    fn record_rate_limit(&self, deployment_id: &str, status: u16) {
        if let Ok(mut recorded) = self.recorded.lock() {
            recorded.push((deployment_id.to_owned(), status));
        }
    }
}
