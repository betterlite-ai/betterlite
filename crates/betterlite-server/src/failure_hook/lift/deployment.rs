use std::collections::BTreeSet;

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;

use super::super::ports::DeploymentCatalog;

pub const PROXY_REJECTED_BEFORE_ROUTING: &str = "proxy_rejected_before_routing";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeploymentAttribution {
    pub custom_llm_provider: Option<String>,
    pub model_info: Option<JsonMap>,
    pub deployment: Option<String>,
}

pub fn deployment_attribution(
    model_group: Option<&str>,
    team_id: Option<&str>,
    catalog: &dyn DeploymentCatalog,
) -> DeploymentAttribution {
    let Some(group) = model_group else {
        return DeploymentAttribution::default();
    };
    let deployments = catalog.get_model_list(group, team_id);
    if deployments.is_empty() {
        return DeploymentAttribution::default();
    }
    let providers: BTreeSet<Option<String>> = deployments
        .iter()
        .map(|deployment| deployment.provider.clone())
        .collect();
    let custom_llm_provider = if providers.len() == 1 {
        providers.into_iter().next().flatten()
    } else {
        None
    };
    let single = if deployments.len() == 1 {
        deployments.first()
    } else {
        None
    };
    let model_info = single.map(|found| found.model_info.clone());
    let deployment = single.map(|found| found.model.clone());
    DeploymentAttribution {
        custom_llm_provider,
        model_info,
        deployment,
    }
}

pub fn stamp_deployment_attribution(
    litellm_params: &mut JsonMap,
    model_group: Option<&str>,
    team_id: Option<&str>,
    dispatched: bool,
    catalog: &dyn DeploymentCatalog,
) -> DeploymentAttribution {
    let attribution = deployment_attribution(model_group, team_id, catalog);
    if let Some(provider) = &attribution.custom_llm_provider {
        litellm_params.insert(
            "custom_llm_provider".to_owned(),
            JsonValue::String(provider.clone()),
        );
    }
    if dispatched {
        return attribution;
    }
    litellm_params.insert(
        PROXY_REJECTED_BEFORE_ROUTING.to_owned(),
        JsonValue::Bool(true),
    );
    let Some(model_info) = &attribution.model_info else {
        return attribution;
    };
    if litellm_params
        .get("metadata")
        .is_none_or(JsonValue::is_null)
    {
        litellm_params.insert("metadata".to_owned(), JsonValue::Object(JsonMap::new()));
    }
    let Some(JsonValue::Object(metadata)) = litellm_params.get_mut("metadata") else {
        return attribution;
    };
    metadata
        .entry("model_info".to_owned())
        .or_insert_with(|| JsonValue::Object(model_info.clone()));
    if let Some(deployment) = &attribution.deployment {
        metadata
            .entry("deployment".to_owned())
            .or_insert_with(|| JsonValue::String(deployment.clone()));
    }
    if let Some(group) = model_group {
        metadata
            .entry("model_group".to_owned())
            .or_insert_with(|| JsonValue::String(group.to_owned()));
    }
    attribution
}
