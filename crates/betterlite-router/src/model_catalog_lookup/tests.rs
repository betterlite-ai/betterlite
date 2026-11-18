#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::deployment_catalog::GroupAlias;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_catalog::error::CatalogLookupError;
use crate::deployment_catalog::scope::litellm_model;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::deployment_identity::stable_id::generate_model_id;
use crate::model_catalog_lookup::alias::{AliasMap, AliasTarget, from_config};
use crate::model_catalog_lookup::cache::{CacheSlot, CatalogCaches, Invalidation};
use crate::model_catalog_lookup::error::LookupError;
use crate::model_catalog_lookup::index::{deployment_ids, rebuild_ids};
use crate::model_catalog_lookup::listing::{get_all_deployments, should_include_deployment};
use crate::model_catalog_lookup::lookup::CatalogLookup;
use crate::routing_groups::group::RoutingGroup;
use crate::routing_groups::index::RoutingGroupIndex;
use crate::wildcard_catalog::Candidate;

mod cache;
mod index;
mod listing;
mod lookup;

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn object(entries: Vec<(&str, Json)>) -> JsonObject {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn deployment(name: &str, provider_model: &str, model_info: JsonObject) -> DeploymentSpec {
    DeploymentSpec::new(
        name.to_owned(),
        object(vec![("model", text(provider_model))]),
        model_info,
        JsonObject::new(),
        String::new,
    )
    .unwrap_or_else(|error| panic!("the fixture deployment is invalid: {error}"))
}

fn named(name: &str, id: &str, provider_model: &str) -> DeploymentSpec {
    deployment(name, provider_model, object(vec![("id", text(id))]))
}

fn team_named(name: &str, id: &str, team: &str, public: &str) -> DeploymentSpec {
    deployment(
        name,
        "openai/team",
        object(vec![
            ("id", text(id)),
            ("team_id", text(team)),
            ("team_public_model_name", text(public)),
        ]),
    )
}

fn priced(name: &str, id: &str, rate: &str) -> DeploymentSpec {
    let params: JsonObject = serde_json::from_str(&format!(
        r#"{{"model":"openai/gpt-4o","input_cost_per_token":{rate}}}"#
    ))
    .unwrap();
    DeploymentSpec::new(
        name.to_owned(),
        params,
        object(vec![("id", text(id))]),
        JsonObject::new(),
        String::new,
    )
    .unwrap()
}

fn raw(name: &str, info: &str) -> DeploymentSpec {
    DeploymentSpec {
        model_name: name.to_owned(),
        litellm_params: object(vec![("model", text("openai/gpt-4o"))]),
        model_info: serde_json::from_str(info).unwrap(),
        extra: JsonObject::new(),
    }
}

fn catalog(rows: &[DeploymentSpec]) -> DeploymentCatalog {
    let mut catalog = DeploymentCatalog::default();
    for row in rows {
        catalog.model_names.insert(row.model_name.clone());
        catalog.index_patterns(row).unwrap();
        catalog.push(row.clone());
    }
    catalog
}

fn ids(rows: &[DeploymentSpec]) -> Vec<&str> {
    rows.iter().filter_map(DeploymentSpec::id).collect()
}

fn names(rows: &[DeploymentSpec]) -> Vec<&str> {
    rows.iter().map(|row| row.model_name.as_str()).collect()
}

fn provider_models(rows: &[DeploymentSpec]) -> Vec<&str> {
    rows.iter().map(|row| litellm_model(row)).collect()
}

struct Snapshot {
    catalog: DeploymentCatalog,
    aliases: AliasMap,
    groups: RoutingGroupIndex,
    provider: Box<dyn Fn(&str) -> Option<String>>,
}

impl Snapshot {
    fn new(rows: &[DeploymentSpec]) -> Self {
        Self {
            catalog: catalog(rows),
            aliases: AliasMap::new(),
            groups: RoutingGroupIndex::default(),
            provider: Box::new(|_: &str| None),
        }
    }

    fn lookup(&self) -> CatalogLookup<'_> {
        CatalogLookup {
            catalog: &self.catalog,
            model_group_alias: &self.aliases,
            routing_groups: &self.groups,
            inferred_provider: &*self.provider,
        }
    }

    fn with_alias(mut self, alias: &str, target: AliasTarget) -> Self {
        self.aliases.insert(alias.to_owned(), target);
        self
    }

    fn with_group(mut self, name: &str, members: &[&str]) -> Self {
        let group = RoutingGroup::new(
            name.to_owned(),
            members.iter().map(|member| (*member).to_owned()).collect(),
            "simple-shuffle",
            None,
            None,
        )
        .unwrap();
        self.groups = RoutingGroupIndex::build(vec![group], &BTreeSet::new(), &BTreeMap::new())
            .unwrap()
            .index;
        self
    }

    fn with_provider(mut self, provider: &str) -> Self {
        let resolved = provider.to_owned();
        self.provider = Box::new(move |_: &str| Some(resolved.clone()));
        self
    }
}

struct Slots {
    info: CacheSlot<String>,
    deployment: CacheSlot<String>,
    zero_cost: CacheSlot<bool>,
    rows: CacheSlot<String>,
    access: CacheSlot<String>,
}

impl Slots {
    fn filled() -> Self {
        let slots = Self {
            info: CacheSlot::default(),
            deployment: CacheSlot::default(),
            zero_cost: CacheSlot::default(),
            rows: CacheSlot::default(),
            access: CacheSlot::default(),
        };
        *slots.info.0.lock().unwrap() = Some("stale".to_owned());
        *slots.deployment.0.lock().unwrap() = Some("stale".to_owned());
        *slots.zero_cost.0.lock().unwrap() = Some(true);
        *slots.rows.0.lock().unwrap() = Some("stale".to_owned());
        *slots.access.0.lock().unwrap() = Some("stale".to_owned());
        slots
    }

    fn caches(&self) -> CatalogCaches<'_> {
        CatalogCaches {
            model_group_info: &self.info,
            deployment_model_info: &self.deployment,
            zero_cost: &self.zero_cost,
            routing_group_rows: &self.rows,
            access_groups: &self.access,
        }
    }
}
