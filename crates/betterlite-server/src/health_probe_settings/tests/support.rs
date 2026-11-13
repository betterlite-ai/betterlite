use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use betterlite_config::sections::health_probe_policy::Settings as ProbePolicy;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;

use crate::health_probe_settings::model_info::HealthCheckModelInfo;
use crate::health_probe_settings::model_settings::ModelHealthSettings;

pub(super) fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).unwrap()
}

pub(super) fn spec(model_name: &str, id: Option<&str>, model: &str) -> DeploymentSpec {
    let mut params = JsonObject::new();
    params.insert("model".to_owned(), Json::Text(model.to_owned()));
    let mut info = JsonObject::new();
    if let Some(id) = id {
        info.insert("id".to_owned(), Json::Text(id.to_owned()));
    }
    DeploymentSpec {
        model_name: model_name.to_owned(),
        litellm_params: params,
        model_info: info,
        extra: JsonObject::new(),
    }
}

pub(super) fn team_spec(
    model_name: &str,
    id: &str,
    model: &str,
    team: &str,
    public: &str,
) -> DeploymentSpec {
    let mut built = spec(model_name, Some(id), model);
    built
        .model_info
        .insert("team_id".to_owned(), Json::Text(team.to_owned()));
    built.model_info.insert(
        "team_public_model_name".to_owned(),
        Json::Text(public.to_owned()),
    );
    built
}

#[derive(Default)]
pub(super) struct StubModels {
    pub(super) modes: BTreeMap<String, String>,
    pub(super) reasoning: BTreeSet<String>,
}

impl StubModels {
    pub(super) fn with_mode(model: &str, mode: &str) -> Self {
        let mut modes = BTreeMap::new();
        modes.insert(model.to_owned(), mode.to_owned());
        Self {
            modes,
            reasoning: BTreeSet::new(),
        }
    }
}

impl HealthCheckModelInfo for StubModels {
    fn mode(&self, model: &str) -> Option<String> {
        self.modes.get(model).cloned()
    }

    fn supports_reasoning(&self, model: &str) -> bool {
        self.reasoning.contains(model)
    }
}

pub(super) fn health(
    max_tokens: Option<i64>,
    max_tokens_reasoning: Option<i64>,
    max_tokens_non_reasoning: Option<i64>,
    supports_max_tokens: Option<bool>,
) -> ModelHealthSettings {
    ModelHealthSettings {
        mode: None,
        max_tokens,
        max_tokens_reasoning,
        max_tokens_non_reasoning,
        supports_max_tokens,
        reasoning_effort: None,
        model: None,
        voice: None,
        params: None,
    }
}

pub(super) fn policy(
    background_max_tokens: Option<i64>,
    background_max_tokens_reasoning: Option<i64>,
) -> ProbePolicy {
    ProbePolicy {
        skip_disabled_background_models: false,
        background_model_groups: None,
        background_max_tokens,
        background_max_tokens_reasoning,
        timeout: Duration::from_secs(60),
        prompt: "test from litellm".to_owned(),
    }
}
