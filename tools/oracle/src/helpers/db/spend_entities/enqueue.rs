use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_db::spend_attribution::access_groups::ServedDeployments;
use betterlite_db::spend_attribution::enqueue::{
    EnqueueFuture, EntitySpendQueue, OwnerKind, RowKind, update_model_access_group_db,
    update_owner_db, update_row_db, update_tag_db,
};
use betterlite_db::spend_attribution::tags::RequestTags;
use betterlite_db::spend_queues::SpendQueueError;
use betterlite_db::spend_queues::entity::{EntityType, SpendAmount, SpendUpdateQueueItem};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::HelperError;
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::Value;

use super::{Step, failure};

#[derive(Debug, Serialize)]
struct EnqueueView {
    queued: Vec<QueuedView>,
    raised: bool,
}

#[derive(Debug, Serialize)]
struct QueuedView {
    entity_type: String,
    entity_id: String,
    response_cost: Option<String>,
}

struct RecordingQueue {
    items: Mutex<Vec<SpendUpdateQueueItem>>,
    fail_at: Option<usize>,
}

impl RecordingQueue {
    fn items(&self) -> Vec<SpendUpdateQueueItem> {
        self.items
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl EntitySpendQueue for RecordingQueue {
    fn enqueue(&self, update: SpendUpdateQueueItem) -> EnqueueFuture<'_> {
        Box::pin(async move {
            let mut items = self.items.lock().unwrap_or_else(PoisonError::into_inner);
            if self.fail_at == Some(items.len()) {
                return Err(SpendQueueError::ZeroCapacity);
            }
            items.push(update);
            drop(items);
            Ok(())
        })
    }
}

struct CaseDeployments {
    deployments: BTreeMap<String, JsonObject>,
}

impl ServedDeployments for CaseDeployments {
    fn model_info(&self, deployment_id: &str) -> Option<&JsonObject> {
        match self.deployments.get(deployment_id)?.get("model_info") {
            Some(Json::Object(model_info)) => Some(model_info),
            _ => None,
        }
    }
}

// EnqueueCase: the writer state an enqueue step needs, so every entity arm reads the same four values.
struct EnqueueCase<'call> {
    step: &'call Step,
    database: Option<&'call dyn EntitySpendQueue>,
    cost: Option<SpendAmount>,
    runtime: &'call tokio::runtime::Runtime,
}

pub(super) fn enqueue_value(step: &Step) -> Result<Value, Box<HelperError>> {
    let queue = RecordingQueue {
        items: Mutex::new(Vec::new()),
        fail_at: step.fail_at,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(failure)?;
    let database: Option<&dyn EntitySpendQueue> = step.database.unwrap_or(true).then_some(&queue);
    let case = EnqueueCase {
        step,
        database,
        cost: spend_amount(step.response_cost.as_ref())?,
        runtime: &runtime,
    };
    let raised = dispatch(&case)?;
    let queued = queue.items().iter().map(queued_view).collect();
    serde_json::to_value(EnqueueView { queued, raised }).map_err(failure)
}

// Entity: the owner or row a spend update charges, each routed to its own writer call.
#[derive(Clone, Copy)]
enum Entity {
    Team,
    Organization,
    Project,
    Agent,
}

// dispatch: the one entity queue call LiteLLM's writer makes for this step, and whether it failed.
fn dispatch(case: &EnqueueCase<'_>) -> Result<bool, Box<HelperError>> {
    match case.step.entity.as_deref().unwrap_or_default() {
        "team" => Ok(entity_raised(case, Entity::Team)),
        "org" => Ok(entity_raised(case, Entity::Organization)),
        "project" => Ok(entity_raised(case, Entity::Project)),
        "agent" => Ok(entity_raised(case, Entity::Agent)),
        "tag" => Ok(tag_raised(case)),
        "model_access_group" => Ok(access_group_raised(case)),
        other => Err(failure(format!("unknown spend_entities entity: {other}"))),
    }
}

// entity_raised: _update_team_db, _update_org_db, _update_project_db or _update_agent_db, whose queue failures reach the caller.
fn entity_raised(case: &EnqueueCase<'_>, entity: Entity) -> bool {
    let step = case.step;
    let database = case.database;
    let cost = case.cost;
    let queued = match entity {
        Entity::Team => case.runtime.block_on(update_owner_db(
            database,
            cost,
            step.team_id.as_deref(),
            step.user_id.as_deref(),
            OwnerKind::Team,
        )),
        Entity::Organization => case.runtime.block_on(update_owner_db(
            database,
            cost,
            step.org_id.as_deref(),
            step.user_id.as_deref(),
            OwnerKind::Organization,
        )),
        Entity::Project => case.runtime.block_on(update_row_db(
            database,
            cost,
            step.project_id.as_deref(),
            RowKind::Project,
        )),
        Entity::Agent => case.runtime.block_on(update_row_db(
            database,
            cost,
            step.agent_id.as_deref(),
            RowKind::Agent,
        )),
    };
    queued.is_err()
}

// tag_raised: _update_tag_db of the writer, whose queue failures reach the caller.
fn tag_raised(case: &EnqueueCase<'_>) -> bool {
    let tags = case
        .step
        .request_tags
        .as_deref()
        .map(RequestTags::Serialized);
    let queued = case
        .runtime
        .block_on(update_tag_db(case.database, case.cost, tags));
    queued.is_err()
}

// access_group_raised: _update_model_access_group_db logs and swallows every queue failure, so the step never raises.
fn access_group_raised(case: &EnqueueCase<'_>) -> bool {
    let attributed = case.step.attributed.clone().unwrap_or_default();
    let deployments = CaseDeployments {
        deployments: case.step.deployments.clone().unwrap_or_default(),
    };
    case.runtime.block_on(update_model_access_group_db(
        case.database,
        case.cost,
        &attributed,
        case.step.served_model_id.as_deref(),
        Some(&deployments),
    ));
    false
}

fn queued_view(item: &SpendUpdateQueueItem) -> QueuedView {
    QueuedView {
        entity_type: item
            .entity_type
            .as_ref()
            .map_or_else(String::new, kind_name),
        entity_id: item.entity_id.clone().unwrap_or_default(),
        response_cost: item.response_cost.map(|amount| format!("{:.10}", amount.0)),
    }
}

fn kind_name(entity_type: &EntityType) -> String {
    match entity_type {
        EntityType::Key => "key",
        EntityType::User => "user",
        EntityType::EndUser => "end_user",
        EntityType::Team => "team",
        EntityType::TeamMember => "team_member",
        EntityType::Organization => "organization",
        EntityType::OrganizationMember => "organization_member",
        EntityType::Project => "project",
        EntityType::Tag => "tag",
        EntityType::Agent => "agent",
        EntityType::ModelAccessGroup => "model_access_group",
        EntityType::Proxy => "proxy",
        EntityType::Unsupported(name) => name.as_str(),
    }
    .to_owned()
}

fn spend_amount(value: Option<&Value>) -> Result<Option<SpendAmount>, Box<HelperError>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(amount) => serde_json::from_value::<Decimal>(amount.clone())
            .map(|spend| Some(SpendAmount(spend)))
            .map_err(failure),
    }
}
