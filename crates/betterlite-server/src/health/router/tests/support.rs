use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::alias::{AliasMap, AliasTarget};
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::routing_groups::index::RoutingGroupIndex;

use crate::health::router::Endpoint;

// The inferred provider the fixture reads through, which never prefixes a wildcard lookup.
type ProviderFn = Box<dyn Fn(&str) -> Option<String>>;

// A catalog as the health run reads it: every deployment under its indices, the alias map beside them and no routing groups.
pub(crate) struct Fixture {
    pub(crate) catalog: DeploymentCatalog,
    pub(crate) aliases: AliasMap,
    pub(crate) groups: RoutingGroupIndex,
    provider: ProviderFn,
}

impl Fixture {
    pub(crate) fn new(deployments: &[DeploymentSpec]) -> Self {
        Self::with_aliases(deployments, &[])
    }

    pub(crate) fn with_aliases(row_list: &[DeploymentSpec], aliases: &[(&str, &str)]) -> Self {
        let mut catalog = DeploymentCatalog::default();
        for row in row_list {
            catalog.model_names.insert(row.model_name.clone());
            catalog
                .index_patterns(row)
                .unwrap_or_else(|error| panic!("the fixture deployment is not indexable: {error}"));
            catalog.push(row.clone());
        }
        let alias_map = aliases
            .iter()
            .map(|(alias, target)| ((*alias).to_owned(), AliasTarget::Name((*target).to_owned())))
            .collect();
        Self {
            catalog,
            aliases: alias_map,
            groups: RoutingGroupIndex::default(),
            provider: Box::new(|_: &str| None),
        }
    }

    pub(crate) fn lookup(&self) -> CatalogLookup<'_> {
        CatalogLookup {
            catalog: &self.catalog,
            model_group_alias: &self.aliases,
            routing_groups: &self.groups,
            inferred_provider: &*self.provider,
        }
    }
}

pub(crate) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(crate) fn object(entries: Vec<(&str, Json)>) -> JsonObject {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

// A deployment whose litellm_params are written as the JSON a proxy config would hold, so a router config reads as it is declared.
pub(crate) fn deployment(name: &str, id: &str, params: &str) -> DeploymentSpec {
    let parsed: JsonObject = serde_json::from_str(params)
        .unwrap_or_else(|error| panic!("the fixture params are not valid JSON: {error}"));
    DeploymentSpec::new(
        name.to_owned(),
        parsed,
        object(vec![("id", text(id))]),
        JsonObject::new(),
        String::new,
    )
    .unwrap_or_else(|error| panic!("the fixture deployment is invalid: {error}"))
}

pub(crate) fn ids(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(crate) fn healthy(id: &str) -> Endpoint {
    Endpoint {
        model_id: Some(id.to_owned()),
        error: None,
    }
}

pub(crate) fn unhealthy(id: &str) -> Endpoint {
    Endpoint {
        model_id: Some(id.to_owned()),
        error: Some("the probe failed".to_owned()),
    }
}

pub(crate) fn names(deployments: &[DeploymentSpec]) -> Vec<&str> {
    deployments
        .iter()
        .map(|deployment| deployment.model_name.as_str())
        .collect()
}
