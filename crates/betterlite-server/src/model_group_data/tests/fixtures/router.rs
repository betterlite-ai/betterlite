use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::catalog_updates::catalog::{CatalogHandle, DeploymentCatalog};
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::alias::{AliasMap, AliasTarget};
use betterlite_router::model_catalog_lookup::error::LookupError;
use betterlite_router::routing_groups::group::RoutingGroup;
use betterlite_router::routing_groups::index::RoutingGroupIndex;

use crate::model_group_data::errors::GroupDataError;
use crate::model_group_data::ports::ModelNameSource;
use crate::route_read_through::ModelReadThrough;

// A router stand-in: whether one is installed, the names it serves, and which a read-through brings live; it mutates itself as the reload rebinding the router would.
pub(crate) struct ScriptedRouter {
    state: Mutex<ScriptedRouterState>,
}

struct ScriptedRouterState {
    present: bool,
    names: BTreeSet<String>,
    read_through_returns: BTreeSet<String>,
    attempted: Vec<String>,
}

impl ScriptedRouter {
    pub(crate) fn new(names: Vec<String>, read_through_returns: Vec<String>) -> Self {
        Self::build(true, names, read_through_returns)
    }

    pub(crate) fn absent(names: Vec<String>, read_through_returns: Vec<String>) -> Self {
        Self::build(false, names, read_through_returns)
    }

    fn build(present: bool, names: Vec<String>, read_through_returns: Vec<String>) -> Self {
        Self {
            state: Mutex::new(ScriptedRouterState {
                present,
                names: names.into_iter().collect(),
                read_through_returns: read_through_returns.into_iter().collect(),
                attempted: Vec::new(),
            }),
        }
    }

    pub(crate) fn attempted(&self) -> Vec<String> {
        let state = self.state.lock().expect("scripted router");
        state.attempted.clone()
    }
}

impl ModelNameSource for ScriptedRouter {
    fn model_names(&self) -> Result<Option<Vec<String>>, GroupDataError> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let present = state.present;
        let names: Vec<String> = state.names.iter().cloned().collect();
        drop(state);
        Ok(present.then_some(names))
    }
}

#[async_trait]
impl ModelReadThrough for ScriptedRouter {
    async fn attempt_model(&self, model: &str) -> bool {
        let mut state = self.state.lock().expect("scripted router");
        state.attempted.push(model.to_owned());
        if !state.read_through_returns.contains(model) {
            return false;
        }
        state.present = true;
        state.names.insert(model.to_owned());
        true
    }
}

fn catalog_deployment(model_name: &str) -> DeploymentSpec {
    DeploymentSpec {
        model_name: model_name.to_owned(),
        litellm_params: JsonObject::new(),
        model_info: JsonObject::new(),
        extra: JsonObject::new(),
    }
}

fn catalog(model_names: &[&str]) -> DeploymentCatalog {
    let mut catalog = DeploymentCatalog::default();
    for model_name in model_names {
        catalog.push(catalog_deployment(model_name));
    }
    catalog
}

// The router catalog an installed router hands every lookup, behind the swap a reload publishes a new one through.
pub(crate) fn catalog_handle(model_names: &[&str]) -> CatalogHandle {
    CatalogHandle::from_pointee(catalog(model_names))
}

// The reload's swap: every later read sees only the names given here.
pub(crate) fn replace_catalog(handle: &CatalogHandle, model_names: &[&str]) {
    handle.store(Arc::new(catalog(model_names)));
}

pub(crate) fn alias_map(entries: &[(&str, AliasTarget)]) -> AliasMap {
    entries
        .iter()
        .map(|(alias, target)| ((*alias).to_owned(), target.clone()))
        .collect()
}

pub(crate) fn routing_groups(group_name: &str, members: &[&str]) -> RoutingGroupIndex {
    let group = RoutingGroup::new(
        group_name.to_owned(),
        members.iter().map(|member| (*member).to_owned()).collect(),
        "simple-shuffle",
        None,
        None,
    )
    .expect("the routing group");
    RoutingGroupIndex::build(vec![group], &BTreeSet::new(), &BTreeMap::new())
        .expect("the routing group index")
        .index
}

// A read-through that installs the model into the live catalog as the registry read-through does, so a recheck over the same source sees it.
pub(crate) struct InstallingReadThrough<'catalog> {
    catalog: &'catalog CatalogHandle,
    installs: BTreeSet<String>,
    attempted: Mutex<Vec<String>>,
}

impl<'catalog> InstallingReadThrough<'catalog> {
    pub(crate) fn new(catalog: &'catalog CatalogHandle, installs: &[&str]) -> Self {
        Self {
            catalog,
            installs: installs.iter().map(|model| (*model).to_owned()).collect(),
            attempted: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn attempted(&self) -> Vec<String> {
        let attempted = self.attempted.lock().expect("installing read-through");
        attempted.clone()
    }
}

#[async_trait]
impl ModelReadThrough for InstallingReadThrough<'_> {
    async fn attempt_model(&self, model: &str) -> bool {
        self.attempted
            .lock()
            .expect("installing read-through")
            .push(model.to_owned());
        if !self.installs.contains(model) {
            return false;
        }
        let served = self.catalog.load();
        let mut next = DeploymentCatalog::default();
        for deployment in &served.models {
            next.push(deployment.clone());
        }
        drop(served);
        next.push(catalog_deployment(model));
        self.catalog.store(Arc::new(next));
        true
    }
}

// A catalog whose index is already stale, the lookup failure a validation must propagate.
pub(crate) struct FailingRouter;

impl ModelNameSource for FailingRouter {
    fn model_names(&self) -> Result<Option<Vec<String>>, GroupDataError> {
        Err(GroupDataError::Catalog(LookupError::InvalidRow {
            index: 3,
        }))
    }
}
