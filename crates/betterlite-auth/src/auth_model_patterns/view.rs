use std::collections::BTreeMap;
use std::fmt::Debug;

use indexmap::IndexMap;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Deployment {
    pub model_name: String,
    pub access_groups: Vec<String>,
    pub team_id: Option<String>,
    pub team_public_model_name: Option<String>,
}

// RouterModelGroupAliasItem: the plain string form is an item with hidden false
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelGroupAlias {
    pub model: String,
    pub hidden: bool,
}

pub trait ModelAccessView: Debug + Send + Sync {
    fn access_groups(&self, model: &str, team_id: Option<&str>) -> BTreeMap<String, Vec<String>>;

    fn model_group_alias_target(&self, model: &str) -> Option<String>;

    fn has_deployment(&self, model_name: &str) -> bool;

    fn model_names(&self) -> Vec<String>;
}

// the router facts the model checks read; routing groups stay out of this view until a caller registers them
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeploymentView {
    pub deployments: Vec<Deployment>,
    pub model_group_alias: IndexMap<String, ModelGroupAlias>,
}

impl DeploymentView {
    // Router._get_all_deployments: a team's own public name wins over every exact-name row
    #[must_use]
    pub fn deployed(&self, model_name: &str, team_id: Option<&str>) -> Vec<&Deployment> {
        if let Some(team_id) = team_id {
            let owned: Vec<&Deployment> = self
                .deployments
                .iter()
                .filter(|deployment| {
                    deployment.team_id.as_deref() == Some(team_id)
                        && deployment.team_public_model_name.as_deref() == Some(model_name)
                })
                .collect();
            if !owned.is_empty() {
                return owned;
            }
        }
        self.deployments
            .iter()
            .filter(|deployment| should_include(model_name, deployment, team_id))
            .collect()
    }

    // Router.get_model_list(model_name, team_id): the listed name and the deployment behind it
    #[must_use]
    pub fn model_rows(&self, model: &str, team_id: Option<&str>) -> Vec<(String, &Deployment)> {
        let mut rows: Vec<(String, &Deployment)> = Vec::new();
        for deployment in self.deployed(model, team_id) {
            rows.push((deployment.model_name.clone(), deployment));
        }
        // the alias's own rows are listed under the alias name
        if let Some(target) = self.model_group_alias_target(model) {
            for deployment in self.deployed(&target, None) {
                rows.push((model.to_owned(), deployment));
            }
        }
        rows
    }

    fn listing_rows(&self) -> Vec<(String, &Deployment)> {
        let mut rows: Vec<(String, &Deployment)> = Vec::new();
        for (alias, entry) in &self.model_group_alias {
            if entry.hidden {
                continue;
            }
            rows.extend(
                self.deployed(&entry.model, None)
                    .into_iter()
                    .map(|deployment| (alias.clone(), deployment)),
            );
        }
        for deployment in &self.deployments {
            rows.push((deployment.model_name.clone(), deployment));
        }
        rows
    }
}

impl ModelAccessView for DeploymentView {
    fn access_groups(&self, model: &str, team_id: Option<&str>) -> BTreeMap<String, Vec<String>> {
        let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (name, deployment) in self.model_rows(model, team_id) {
            for group in &deployment.access_groups {
                groups.entry(group.clone()).or_default().push(name.clone());
            }
        }
        groups
    }

    fn model_group_alias_target(&self, model: &str) -> Option<String> {
        self.model_group_alias
            .get(model)
            .map(|entry| entry.model.as_str())
            .filter(|target| !target.is_empty())
            .map(str::to_owned)
    }

    fn has_deployment(&self, model_name: &str) -> bool {
        self.deployments
            .iter()
            .any(|deployment| deployment.model_name == model_name)
    }

    // Router.get_model_names: a team-specific row yields no name without a team_id
    fn model_names(&self) -> Vec<String> {
        self.listing_rows()
            .into_iter()
            .filter(|(_, deployment)| deployment.team_id.is_none())
            .map(|(name, _)| name)
            .collect()
    }
}

fn should_include(model_name: &str, deployment: &Deployment, team_id: Option<&str>) -> bool {
    if let Some(team_id) = team_id
        && deployment.team_id.as_deref() == Some(team_id)
        && deployment.team_public_model_name.as_deref() == Some(model_name)
    {
        return true;
    }
    if deployment.model_name != model_name {
        return false;
    }
    match team_id {
        None => true,
        Some(team_id) => deployment
            .team_id
            .as_deref()
            .is_none_or(|deployment_team| deployment_team == team_id),
    }
}
