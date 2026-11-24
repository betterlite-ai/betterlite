use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::deployment_identity::stable_id::generate_model_id;
use betterlite_router::model_catalog_lookup::alias::AliasMap;
use betterlite_router::model_group_info::group::ModelGroupInfo;
use betterlite_router::routing_groups::index::RoutingGroupIndex;

use super::error::Failure;
use super::input::{GroupInfoInput, Row};

pub(crate) struct Snapshot {
    pub(crate) catalog: DeploymentCatalog,
    pub(crate) aliases: AliasMap,
    pub(crate) groups: RoutingGroupIndex,
}

impl Snapshot {
    // Router(model_list=...): every row as a deployment under the catalog's indices.
    pub(crate) fn build(rows: &[Row]) -> Result<Self, Failure> {
        let mut catalog = DeploymentCatalog::default();
        for row in rows {
            let spec = spec(row)?;
            catalog.index_patterns(&spec)?;
            catalog.push(spec);
        }
        Ok(Self {
            catalog,
            aliases: AliasMap::new(),
            groups: RoutingGroupIndex::default(),
        })
    }

    // Router.get_model_list(model_name=group): the group's own deployments, in model-list order.
    pub(crate) fn rows(&self, group: &str) -> Vec<DeploymentSpec> {
        self.catalog
            .models
            .iter()
            .filter(|deployment| deployment.model_name == group)
            .cloned()
            .collect()
    }
}

// The router's per-row identity: the declared id, or the stable one derived from the name and params.
fn spec(row: &Row) -> Result<DeploymentSpec, Failure> {
    let mut model_info = row.model_info.clone();
    if !model_info.contains_key("id") {
        let generated = generate_model_id(&row.model_name, &row.litellm_params);
        model_info.insert("id".to_owned(), Json::Text(generated));
    }
    Ok(DeploymentSpec::new(
        row.model_name.clone(),
        row.litellm_params.clone(),
        model_info,
        row.extra.clone(),
        String::new,
    )?)
}

// Router._cached_get_model_group_info's answer: the four limits the remaining-usage read asks for.
pub(crate) fn group_info(input: Option<&GroupInfoInput>) -> Option<ModelGroupInfo> {
    let info = input?;
    Some(ModelGroupInfo {
        model_group: String::new(),
        providers: Vec::new(),
        max_input_tokens: None,
        max_output_tokens: None,
        input_cost_per_token: None,
        output_cost_per_token: None,
        input_cost_per_pixel: None,
        mode: None,
        tpm: info.tpm,
        rpm: info.rpm,
        itpm: info.itpm,
        otpm: info.otpm,
        capabilities: BTreeSet::new(),
        supports_fast_mode: false,
        supported_reasoning_efforts: None,
        supported_openai_params: None,
        clientside_auth: None,
    })
}
