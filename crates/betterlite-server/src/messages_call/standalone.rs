use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;
use betterlite_error::proxy_model_not_found::{ProxyModelNotFoundError, route_endpoint};
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::{ChatError, ChatExecutor};
use betterlite_openai::openai_base_config::Object as JsonMap;
use secrecy::ExposeSecret as _;

use super::MessagesCallError;
use super::outcome::{MessagesDispatch, MessagesOutcome, MessagesRouting};
use crate::keepalive_interval::DeploymentLookup;
use crate::routing::simple_shuffle::{pick_deployment, pick_matching};

struct StandaloneRouting {
    route_type: &'static str,
    executor: Arc<ChatExecutor>,
    dispatch: Arc<dyn MessagesDispatch>,
    model_names: BTreeSet<String>,
    deployment_names: BTreeSet<String>,
}

// What B081's routes(state) builds until the production router replaces it
#[must_use]
pub fn standalone_routing(
    route_type: &'static str,
    executor: Arc<ChatExecutor>,
    dispatch: Arc<dyn MessagesDispatch>,
) -> Arc<dyn MessagesRouting> {
    let model_names: BTreeSet<String> = executor
        .deployments
        .iter()
        .map(|deployment| deployment.model_group.clone())
        .collect();
    let deployment_names: BTreeSet<String> = executor
        .deployments
        .iter()
        .map(|deployment| deployment.litellm_model.clone())
        .collect();
    Arc::new(StandaloneRouting {
        route_type,
        executor,
        dispatch,
        model_names,
        deployment_names,
    })
}

impl DeploymentLookup for StandaloneRouting {
    // The empty list is the whole answer, so every mutant of it is the same empty list
    #[cfg_attr(test, mutants::skip)]
    fn deployments_for(&self, _model_name: &str) -> Vec<JsonValue> {
        Vec::new()
    }
}

#[async_trait]
impl MessagesRouting for StandaloneRouting {
    fn all_blocked(&self, _model: &str, _team_id: Option<&str>) -> bool {
        false
    }

    fn map_team_model(&self, _model: Option<&str>, _team_id: &str) -> Option<String> {
        None
    }

    fn is_recognized_model(&self, model: &str) -> bool {
        self.model_names.contains(model)
    }

    fn model_names_has(&self, model: &str) -> bool {
        self.model_names.contains(model)
    }

    fn team_public_model_names_has(&self, _model: &str) -> bool {
        false
    }

    fn pass_through_all_models(&self) -> bool {
        false
    }

    fn has_default_or_pattern(&self) -> bool {
        false
    }

    fn is_deployment_name(&self, model: &str) -> bool {
        self.deployment_names.contains(model)
    }

    async fn anthropic_messages(
        &self,
        mut data: JsonMap,
        specific_deployment: bool,
    ) -> Result<MessagesOutcome, MessagesCallError> {
        let model = data
            .get("model")
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_owned();
        let deployment =
            choose_deployment(&self.executor, &model, specific_deployment).map_err(|_error| {
                MessagesCallError::NotFound(ProxyModelNotFoundError::new(
                    route_endpoint(self.route_type).unwrap_or(self.route_type),
                    &model,
                    true,
                ))
            })?;
        write_deployment(&mut data, deployment);
        let mut outcome = self.dispatch.dispatch(data).await?;
        let hidden = outcome.hidden_mut();
        hidden.insert(
            "model_id".to_owned(),
            JsonValue::String(deployment.model_id.clone()),
        );
        hidden.insert(
            "api_base".to_owned(),
            JsonValue::String(deployment.api_base.clone()),
        );
        Ok(outcome)
    }

    async fn a2a_agent(
        &self,
        _data: JsonMap,
    ) -> Option<Result<MessagesOutcome, MessagesCallError>> {
        None
    }
}

fn choose_deployment<'executor>(
    executor: &'executor ChatExecutor,
    model: &str,
    specific_deployment: bool,
) -> Result<&'executor ChatDeployment, ChatError> {
    if specific_deployment {
        let matching: Vec<&ChatDeployment> = executor
            .deployments
            .iter()
            .filter(|deployment| deployment.litellm_model == model)
            .collect();
        return pick_matching(executor, model, &matching, "a deployment name");
    }
    pick_deployment(executor, model)
}

// Router hands the picked deployment's wire settings to the provider call
fn write_deployment(data: &mut JsonMap, deployment: &ChatDeployment) {
    data.insert(
        "model".to_owned(),
        JsonValue::String(deployment.litellm_model.clone()),
    );
    data.insert(
        "api_base".to_owned(),
        JsonValue::String(deployment.api_base.clone()),
    );
    if let Some(api_key) = &deployment.api_key {
        data.insert(
            "api_key".to_owned(),
            JsonValue::String(api_key.expose_secret().to_owned()),
        );
    }
    data.insert(
        "timeout".to_owned(),
        JsonValue::from(deployment.timeout_seconds),
    );
    write_model_info(data, &deployment.model_id);
}

// Router sets litellm_metadata.model_info.id so the provider call finds the deployment it picked
fn write_model_info(data: &mut JsonMap, model_id: &str) {
    let metadata = data
        .entry("litellm_metadata".to_owned())
        .or_insert_with(|| JsonValue::Object(JsonMap::new()));
    if !metadata.is_object() {
        *metadata = JsonValue::Object(JsonMap::new());
    }
    if let JsonValue::Object(fields) = metadata {
        let info = fields
            .entry("model_info".to_owned())
            .or_insert_with(|| JsonValue::Object(JsonMap::new()));
        if !info.is_object() {
            *info = JsonValue::Object(JsonMap::new());
        }
        if let JsonValue::Object(info_fields) = info {
            info_fields.insert("id".to_owned(), JsonValue::String(model_id.to_owned()));
        }
    }
}
