use std::collections::BTreeMap;
use std::sync::Arc;

use indexmap::IndexMap;
use indexmap::map::Entry;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tokio_util::task::TaskTracker;

use super::SpendQueueError;
use super::events::{SpendQueueEvent, SpendQueueEventSink, SpendQueueService};
use super::queue::BoundedUpdateQueue;
use crate::queues::memory::EnqueueObserver;

// Litellm_EntityType of litellm/proxy/_types.py, plus the two kinds the spend map has no bucket for.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(from = "String")]
pub enum EntityType {
    Key,
    User,
    EndUser,
    Team,
    TeamMember,
    Organization,
    OrganizationMember,
    Project,
    Tag,
    Agent,
    ModelAccessGroup,
    Proxy,
    // A kind written by a pod whose Litellm_EntityType has a member this version does not.
    Unsupported(String),
}

impl From<String> for EntityType {
    fn from(name: String) -> Self {
        match name.as_str() {
            "key" => Self::Key,
            "user" => Self::User,
            "end_user" => Self::EndUser,
            "team" => Self::Team,
            "team_member" => Self::TeamMember,
            "organization" => Self::Organization,
            "organization_member" => Self::OrganizationMember,
            "project" => Self::Project,
            "tag" => Self::Tag,
            "agent" => Self::Agent,
            "model_access_group" => Self::ModelAccessGroup,
            "proxy" => Self::Proxy,
            other => Self::Unsupported(other.to_owned()),
        }
    }
}

// The response_cost field of SpendUpdateQueueItem, which is a float in the source and a Decimal here so a sum of costs stays exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SpendAmount(#[serde(with = "rust_decimal::serde::arbitrary_precision")] pub Decimal);

impl SpendAmount {
    pub const ZERO: Self = Self(Decimal::ZERO);
}

// SpendUpdateQueueItem of litellm/proxy/_types.py; every field is optional there.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SpendUpdateQueueItem {
    #[serde(default)]
    pub entity_type: Option<EntityType>,
    #[serde(default)]
    pub entity_id: Option<String>,
    #[serde(default)]
    pub response_cost: Option<SpendAmount>,
}

// DBSpendUpdateTransactions of litellm/proxy/_types.py. Every bucket is built, empty when nothing was queued for it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SpendUpdateTransactions {
    pub user_list_transactions: BTreeMap<String, SpendAmount>,
    pub end_user_list_transactions: BTreeMap<String, SpendAmount>,
    pub key_list_transactions: BTreeMap<String, SpendAmount>,
    pub team_list_transactions: BTreeMap<String, SpendAmount>,
    pub team_member_list_transactions: BTreeMap<String, SpendAmount>,
    pub org_list_transactions: BTreeMap<String, SpendAmount>,
    pub org_member_list_transactions: BTreeMap<String, SpendAmount>,
    pub project_list_transactions: BTreeMap<String, SpendAmount>,
    pub tag_list_transactions: BTreeMap<String, SpendAmount>,
    pub agent_list_transactions: BTreeMap<String, SpendAmount>,
    #[serde(rename = "model_access_group_list_transactions")]
    pub model_access_group_spend: BTreeMap<String, SpendAmount>,
}

impl SpendUpdateTransactions {
    const fn bucket_mut(
        &mut self,
        entity_type: &EntityType,
    ) -> Option<&mut BTreeMap<String, SpendAmount>> {
        match entity_type {
            EntityType::Key => Some(&mut self.key_list_transactions),
            EntityType::User => Some(&mut self.user_list_transactions),
            EntityType::EndUser => Some(&mut self.end_user_list_transactions),
            EntityType::Team => Some(&mut self.team_list_transactions),
            EntityType::TeamMember => Some(&mut self.team_member_list_transactions),
            EntityType::Organization => Some(&mut self.org_list_transactions),
            EntityType::OrganizationMember => Some(&mut self.org_member_list_transactions),
            EntityType::Project => Some(&mut self.project_list_transactions),
            EntityType::Tag => Some(&mut self.tag_list_transactions),
            EntityType::Agent => Some(&mut self.agent_list_transactions),
            EntityType::ModelAccessGroup => Some(&mut self.model_access_group_spend),
            EntityType::Proxy | EntityType::Unsupported(_) => None,
        }
    }
}

// _get_aggregated_spend_update_queue_item: groups by entity type and id in first-seen order, keeps the first record of a group and adds every later cost onto it without touching an input record.
pub fn aggregate_queue_items(
    updates: &[SpendUpdateQueueItem],
) -> Result<Vec<SpendUpdateQueueItem>, SpendQueueError> {
    let mut grouped: IndexMap<(Option<&EntityType>, Option<&str>), SpendUpdateQueueItem> =
        IndexMap::new();
    for update in updates {
        let group = (update.entity_type.as_ref(), update.entity_id.as_deref());
        match grouped.entry(group) {
            Entry::Occupied(mut occupied) => {
                let current = occupied.get_mut();
                let total = response_cost(current)
                    .checked_add(response_cost(update))
                    .ok_or(SpendQueueError::CostOverflow)?;
                current.response_cost = Some(SpendAmount(total));
            }
            Entry::Vacant(vacant) => {
                vacant.insert(update.clone());
            }
        }
    }
    Ok(grouped.into_values().collect())
}

// get_aggregated_db_spend_update_transactions: one bucket per entity kind, keyed by the entity id with an absent id reading as the empty string.
pub fn aggregate_db_transactions(
    updates: &[SpendUpdateQueueItem],
) -> Result<SpendUpdateTransactions, SpendQueueError> {
    let mut transactions = SpendUpdateTransactions::default();
    for update in updates {
        let Some(entity_type) = update.entity_type.as_ref() else {
            tracing::debug!("Skipping update spend because the entity kind is missing");
            continue;
        };
        let Some(bucket) = transactions.bucket_mut(entity_type) else {
            tracing::debug!("Skipping update spend because the entity kind has no spend bucket");
            continue;
        };
        let entity_id = update.entity_id.as_deref().unwrap_or_default();
        let total = bucket
            .entry(entity_id.to_owned())
            .or_insert(SpendAmount::ZERO);
        total.0 = total
            .0
            .checked_add(response_cost(update))
            .ok_or(SpendQueueError::CostOverflow)?;
    }
    Ok(transactions)
}

// response_cost is read as `... or 0` in the source, so a falsey cost adds integer zero rather than its float spelling.
fn response_cost(update: &SpendUpdateQueueItem) -> Decimal {
    let cost = update
        .response_cost
        .map_or(Decimal::ZERO, |amount| amount.0);
    if cost.is_zero() { Decimal::ZERO } else { cost }
}

pub struct SpendUpdateQueue {
    queue: BoundedUpdateQueue<SpendUpdateQueueItem>,
    sink: Arc<dyn SpendQueueEventSink>,
    service_tasks: TaskTracker,
}

impl SpendUpdateQueue {
    pub fn new(
        queue_capacity: i64,
        aggregation_threshold: i64,
        flush_count: i64,
        sink: Arc<dyn SpendQueueEventSink>,
    ) -> Result<Self, SpendQueueError> {
        let queue = BoundedUpdateQueue::new(queue_capacity, aggregation_threshold, flush_count)?;
        Ok(Self {
            queue,
            sink,
            service_tasks: TaskTracker::new(),
        })
    }

    // add_update: admit the record, report the queue size, then compact once the inherited threshold is reached.
    pub async fn add_update(&self, update: SpendUpdateQueueItem) -> Result<(), SpendQueueError> {
        // The source logs the whole update here; betterlite's log line keeps spend bodies out of every log.
        tracing::debug!("Adding update to the in-memory spend queue");
        self.queue.add(update, self).await;
        self.queue.compact_when_full(aggregate_queue_items)
    }

    // flush_and_get_aggregated_db_spend_update_transactions: the entity queue owns no interrupted commits, so the drain is all a flush waits for.
    pub fn flush_aggregated_transactions(
        &self,
    ) -> Result<SpendUpdateTransactions, SpendQueueError> {
        let updates = self.queue.drain();
        if !updates.is_empty() {
            tracing::info!(
                items = updates.len(),
                "Spend tracking - flushed spend update items from in-memory queue"
            );
        }
        aggregate_db_transactions(&updates)
    }

    pub async fn shutdown(&self) {
        self.service_tasks.close();
        self.service_tasks.wait().await;
    }
}

impl EnqueueObserver for SpendUpdateQueue {
    fn item_added(&self, queue_size: usize) {
        let event = SpendQueueEvent {
            service: SpendQueueService::InMemorySpendUpdateQueue,
            queue_size,
        };
        let sink = Arc::clone(&self.sink);
        self.service_tasks.spawn(async move { sink.emit(event) });
    }
}
