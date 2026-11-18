use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::alias::{AliasMap, AliasTarget};
use crate::model_recognition::ModelRecognition;
use crate::routing_groups::group::RoutingGroup;
use crate::routing_groups::index::RoutingGroupIndex;

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

// RoutingGroupIndex._init_routing_groups: the groups plus the names and aliases that shadow them.
pub(super) fn index(
    groups: Vec<RoutingGroup>,
    known: &[&str],
    aliases: &[&str],
) -> RoutingGroupIndex {
    let model_names: BTreeSet<String> = known.iter().map(|name| (*name).to_owned()).collect();
    let alias_map: BTreeMap<String, String> = aliases
        .iter()
        .map(|name| ((*name).to_owned(), (*name).to_owned()))
        .collect();
    RoutingGroupIndex::build(groups, &model_names, &alias_map)
        .unwrap()
        .index
}

// One catalog snapshot with the aliases and group index built beside it.
#[derive(Default)]
pub(super) struct Snapshot {
    pub(super) catalog: DeploymentCatalog,
    pub(super) aliases: AliasMap,
    pub(super) groups: RoutingGroupIndex,
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

    pub(super) fn with_groups(
        mut self,
        groups: Vec<RoutingGroup>,
        known: &[&str],
        aliases: &[&str],
    ) -> Self {
        self.groups = index(groups, known, aliases);
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
