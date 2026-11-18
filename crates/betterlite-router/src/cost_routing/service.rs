use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use indexmap::IndexSet;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::io_token_reservations::clock::IoClock;

use super::costs::ModelCosts;
use super::error::CostRoutingError;
use super::keys::minute_label;
use super::selection::{Selection, select};
use super::window::{CostWindows, WindowUsage};

// is_batch_retrieve_call_type: a batch retrieve reports the whole job's usage on every poll, so it never feeds the map.
const BATCH_RETRIEVE_CALL_TYPES: [&str; 2] = ["aretrieve_batch", "retrieve_batch"];
// LiteLLM's in-memory cache is bounded by its own settings; these caps keep a long-lived router's order index bounded too.
pub(super) const MAX_GROUPS: usize = 1024;
pub(super) const MAX_DEPLOYMENTS_PER_GROUP: usize = 1024;

// What a success callback hands the cost map: the call type, the group and id the deployment was called under, and its usage.
pub struct SuccessEvent<'event> {
    pub call_type: Option<&'event str>,
    pub model_group: Option<&'event str>,
    pub deployment_id: Option<&'event str>,
    pub total_tokens: i64,
}

// One deployment's entry in the map, in first-observation order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
    pub deployment_id: String,
    pub usage: WindowUsage,
}

// cost_map:{model_group}: every deployment id the group has seen, in the order the map first recorded it.
#[derive(Debug, Default)]
struct OrderIndex {
    groups: BTreeMap<String, IndexSet<String>>,
}

impl OrderIndex {
    fn group_mut(&mut self, model_group: &str) -> Option<&mut IndexSet<String>> {
        if !self.groups.contains_key(model_group) {
            if self.groups.len() >= MAX_GROUPS {
                tracing::warn!(
                    model_group,
                    "the cost map group limit is reached; the group is not tracked"
                );
                return None;
            }
            self.groups.insert(model_group.to_owned(), IndexSet::new());
        }
        self.groups.get_mut(model_group)
    }

    fn insert(&mut self, model_group: &str, deployment_id: &str) {
        let Some(entries) = self.group_mut(model_group) else {
            return;
        };
        if entries.len() >= MAX_DEPLOYMENTS_PER_GROUP && !entries.contains(deployment_id) {
            tracing::warn!(
                model_group,
                deployment_id,
                "the cost map deployment limit is reached; the deployment is not tracked"
            );
            return;
        }
        entries.insert(deployment_id.to_owned());
    }

    // The pinned selector adds every healthy deployment it has not seen with this minute's zero usage. The map is only
    // stored once a success writes it; until then each selection builds a fresh dict from the pool and keeps nothing.
    fn ids(&mut self, model_group: &str, deployments: &[DeploymentSpec]) -> Vec<String> {
        let Some(entries) = self.groups.get_mut(model_group) else {
            return pool_ids(deployments);
        };
        for deployment in deployments {
            let Some(deployment_id) = deployment.id() else {
                continue;
            };
            if entries.contains(deployment_id) {
                continue;
            }
            if entries.len() >= MAX_DEPLOYMENTS_PER_GROUP {
                tracing::warn!(
                    model_group,
                    deployment_id,
                    "the cost map deployment limit is reached; the deployment is not tracked"
                );
                continue;
            }
            entries.insert(deployment_id.to_owned());
        }
        entries.iter().cloned().collect()
    }

    fn known(&self, model_group: &str) -> Vec<String> {
        self.groups
            .get(model_group)
            .map(|entries| entries.iter().cloned().collect())
            .unwrap_or_default()
    }
}

fn pool_ids(deployments: &[DeploymentSpec]) -> Vec<String> {
    deployments
        .iter()
        .filter_map(|deployment| deployment.id().map(str::to_owned))
        .collect()
}

// LowestCostLoggingHandler: the router cache the pinned handler is built with is this pair of windows and order.
pub struct CostRouter<W: CostWindows> {
    windows: W,
    clock: IoClock,
    order: Mutex<OrderIndex>,
}

impl<W: CostWindows> CostRouter<W> {
    #[must_use]
    pub fn new(windows: W, clock: IoClock) -> Self {
        Self {
            windows,
            clock,
            order: Mutex::new(OrderIndex::default()),
        }
    }

    // log_success_event and async_log_success_event: one map update, whichever callback LiteLLM ran.
    pub async fn record_success(&self, event: &SuccessEvent<'_>) {
        if event
            .call_type
            .is_some_and(|call_type| BATCH_RETRIEVE_CALL_TYPES.contains(&call_type))
        {
            return;
        }
        let (Some(model_group), Some(deployment_id)) = (event.model_group, event.deployment_id)
        else {
            return;
        };
        let minute = minute_label((self.clock)());
        self.order
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(model_group, deployment_id);
        self.windows
            .record(model_group, deployment_id, &minute, event.total_tokens)
            .await;
    }

    // A route carries no stored window values, so this is the cold-cache answer: every deployment starts unused.
    pub fn select(
        &self,
        costs: &dyn ModelCosts,
        request: &Selection<'_>,
    ) -> Result<Option<usize>, CostRoutingError> {
        let minute = minute_label((self.clock)());
        let order = self
            .order
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .ids(request.group, request.deployments);
        let used: Vec<WindowUsage> = order
            .iter()
            .map(|deployment_id| self.windows.usage(request.group, deployment_id, &minute))
            .collect();
        select(costs, request, &order, &used)
    }

    // The map's entries for one minute, in insertion order, as the cache read the pinned handler makes would show them.
    #[must_use]
    pub fn entries(&self, model_group: &str, minute: &str) -> Vec<Entry> {
        let ids = self
            .order
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .known(model_group);
        ids.into_iter()
            .map(|deployment_id| Entry {
                usage: self.windows.usage(model_group, &deployment_id, minute),
                deployment_id,
            })
            .collect()
    }
}
