use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use betterlite_config::sections::health_probe_policy::Settings as ProbePolicy;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;

use crate::health_probe_settings::model_info::HealthCheckModelInfo;
use crate::health_probe_settings::model_settings::ModelHealthSettings;

pub(super) fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).unwrap()
}

pub(super) fn spec(model_name: &str, id: Option<&str>, model: &str) -> DeploymentSpec {
    let params = object(&format!(r#"{{"model":"{model}"}}"#));
    let info = id.map_or_else(
        || object("{}"),
        |value| object(&format!(r#"{{"id":"{value}"}}"#)),
    );
    DeploymentSpec {
        model_name: model_name.to_owned(),
        litellm_params: params,
        model_info: info,
        extra: JsonObject::new(),
    }
}

pub(super) fn with_team(
    mut deployment: DeploymentSpec,
    team: &str,
    public: &str,
) -> DeploymentSpec {
    deployment.model_info.extend(object(&format!(
        r#"{{"team_id":"{team}","team_public_model_name":"{public}"}}"#
    )));
    deployment
}

#[derive(Default)]
pub(super) struct StubModels {
    pub(super) modes: BTreeMap<String, String>,
    pub(super) reasoning: BTreeSet<String>,
}

impl StubModels {
    pub(super) fn with_mode(model: &str, mode: &str) -> Self {
        let mut by_model = BTreeMap::new();
        by_model.insert(model.to_owned(), mode.to_owned());
        Self {
            modes: by_model,
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

pub(super) fn policy(max_tokens: Option<i64>, max_tokens_reasoning: Option<i64>) -> ProbePolicy {
    ProbePolicy {
        skip_disabled_models: false,
        model_groups: None,
        max_tokens,
        max_tokens_reasoning,
        timeout: Duration::from_secs(60),
        prompt: "test from litellm".to_owned(),
    }
}
