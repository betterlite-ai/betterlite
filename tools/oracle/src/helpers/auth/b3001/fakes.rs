use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_auth::auth_request_models::catalog::{
    ManagedIdDecoders, ManagedIdError, ManagedIdFamily, ModelCatalog,
};
use betterlite_auth::reservation_cost::error::ModelInfoError;
use betterlite_auth::reservation_cost::{
    ModelInfoSource, ReservationTokens, RouterDeployment, RouterModelInfo,
};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;
use serde_json::Number;

use super::input::Deployment;

const PER_TOKEN_RATES: [&str; 2] = ["input_cost_per_token", "output_cost_per_token"];
const GROUP_ONLY_ABSENT_FIELDS: [&str; 3] = [
    "tiered_pricing",
    "input_cost_per_image",
    "output_cost_per_image",
];

// The case's model_cost entry is what litellm.get_model_info reads, so a missing per-token rate reads as zero.
#[derive(Debug)]
pub(crate) struct FakeModelInfo<'costs> {
    pub(crate) entries: &'costs BTreeMap<String, JsonObject>,
}

impl ModelInfoSource for FakeModelInfo<'_> {
    fn get_model_info(&self, model: &str) -> Result<Option<JsonObject>, ModelInfoError> {
        let Some(entry) = self.entries.get(model) else {
            return Err(ModelInfoError::Unavailable);
        };
        let mut info = entry.clone();
        for field in PER_TOKEN_RATES {
            if matches!(info.get(field), None | Some(Json::Null)) {
                info.insert(field.to_owned(), Json::Number(Number::from(0)));
            }
        }
        Ok(Some(info))
    }
}

// The router's cached views: both merge litellm_params.model's catalog entry with the deployment model_info.
#[derive(Debug)]
pub(crate) struct FakeRouter<'cases> {
    pub(crate) entries: &'cases BTreeMap<String, JsonObject>,
    pub(crate) deployments: &'cases [Deployment],
}

impl RouterModelInfo for FakeRouter<'_> {
    fn get_model_list(&self, model_name: &str) -> Result<Vec<RouterDeployment>, ModelInfoError> {
        Ok(self
            .deployments
            .iter()
            .filter(|deployment| deployment.model_name == model_name)
            .map(|deployment| RouterDeployment {
                model_id: text_value(deployment.model_info.get("id")),
                backend_model: Some(deployment.backend_model.clone()),
            })
            .collect())
    }

    fn cached_deployment_model_info(
        &self,
        model_id: &str,
        backend_model: &str,
    ) -> Result<Option<JsonObject>, ModelInfoError> {
        Ok(self
            .deployments
            .iter()
            .find(|deployment| {
                deployment.backend_model == backend_model
                    && text_value(deployment.model_info.get("id")).as_deref() == Some(model_id)
            })
            .map(|deployment| self.merged_info(deployment)))
    }

    fn cached_model_group_info(
        &self,
        model_name: &str,
    ) -> Result<Option<JsonObject>, ModelInfoError> {
        let Some(deployment) = self
            .deployments
            .iter()
            .find(|deployment| deployment.model_name == model_name)
        else {
            return Ok(None);
        };
        let mut info = self.merged_info(deployment);
        for field in GROUP_ONLY_ABSENT_FIELDS {
            info.shift_remove(field);
        }
        fill_paired_rates(&mut info);
        Ok(Some(info))
    }
}

impl FakeRouter<'_> {
    fn merged_info(&self, deployment: &Deployment) -> JsonObject {
        let mut info = self
            .catalog_entry(&deployment.backend_model)
            .cloned()
            .unwrap_or_default();
        for (key, value) in &deployment.model_info {
            info.insert(key.clone(), value.clone());
        }
        info
    }

    fn catalog_entry(&self, backend_model: &str) -> Option<&JsonObject> {
        if let Some(entry) = self.entries.get(backend_model) {
            return Some(entry);
        }
        let (_, stripped) = backend_model.split_once('/')?;
        self.entries.get(stripped)
    }
}

// A group snapshot with one per-token rate declared zero-fills the other; with neither it keeps both absent.
fn fill_paired_rates(info: &mut JsonObject) {
    let [input_field, output_field] = PER_TOKEN_RATES;
    let input_absent = matches!(info.get(input_field), None | Some(Json::Null));
    let output_absent = matches!(info.get(output_field), None | Some(Json::Null));
    if input_absent == output_absent {
        return;
    }
    let missing = if input_absent {
        input_field
    } else {
        output_field
    };
    info.insert(missing.to_owned(), Json::Number(Number::from(0)));
}

fn text_value(value: Option<&Json>) -> Option<String> {
    match value {
        Some(Json::Text(text)) => Some(text.clone()),
        _ => None,
    }
}

// The case rejects a countable body up front: B1695's tokenizer is not part of this helper.
#[derive(Debug)]
pub(crate) struct FakeTokens;

#[async_trait]
impl ReservationTokens for FakeTokens {
    async fn count_input_tokens(
        &self,
        _body: &JsonObject,
        _models: &[String],
    ) -> IndexMap<String, usize> {
        IndexMap::new()
    }

    async fn count_input_tokens_for_model(
        &self,
        _body: &JsonObject,
        _model: &str,
    ) -> Option<usize> {
        None
    }
}

#[derive(Debug)]
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
