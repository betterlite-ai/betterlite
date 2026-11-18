use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::alias::{AliasMap, AliasTarget};
use crate::model_recognition::ModelRecognition;
use crate::routing_groups::group::RoutingGroup;

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn deployment(name: &str, id: &str) -> DeploymentSpec {
    let mut params = JsonObject::new();
    params.insert("model".to_owned(), text("openai/gpt-4o"));
    let mut info = JsonObject::new();
    info.insert("id".to_owned(), text(id));
    DeploymentSpec::new(
        name.to_owned(),
        params,
        info,
        JsonObject::new(),
        String::new,
    )
    .unwrap()
}

pub(super) fn group(name: &str, members: &[&str]) -> RoutingGroup {
    let named = members.iter().map(|member| (*member).to_owned()).collect();
    RoutingGroup::new(name.to_owned(), named, "simple-shuffle", None, None).unwrap()
}

// One snapshot: the catalog, the aliases and Router._routing_groups, every parsed group included.
#[derive(Default)]
pub(super) struct Snapshot {
    pub(super) catalog: DeploymentCatalog,
    pub(super) aliases: AliasMap,
    pub(super) groups: IndexMap<String, RoutingGroup>,
}

impl Snapshot {
    pub(super) fn with_deployment(mut self, name: &str, id: &str) -> Self {
        let spec = deployment(name, id);
        self.catalog.model_names.insert(name.to_owned());
        self.catalog.index_patterns(&spec).unwrap();
        self.catalog.push(spec);
        self
    }

    pub(super) fn with_alias(mut self, name: &str, target: &str) -> Self {
        self.aliases
            .insert(name.to_owned(), AliasTarget::Name(target.to_owned()));
        self
    }

    pub(super) fn with_groups(mut self, groups: Vec<RoutingGroup>) -> Self {
        self.groups = groups
            .into_iter()
            .map(|group| (group.group_name.clone(), group))
            .collect();
        self
    }

    pub(super) fn recognition(&self) -> ModelRecognition<'_> {
        ModelRecognition {
            catalog: &self.catalog,
            model_group_alias: &self.aliases,
            routing_groups: &self.groups,
        }
    }
}
