use std::collections::BTreeMap;
use std::sync::Mutex;

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
    const fn new(fail_at: Option<usize>) -> Self {
        Self {
            items: Mutex::new(Vec::new()),
            fail_at,
        }
    }

    fn items(&self) -> Vec<SpendUpdateQueueItem> {
        self.items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl EntitySpendQueue for RecordingQueue {
    fn enqueue(&self, update: SpendUpdateQueueItem) -> EnqueueFuture<'_> {
        Box::pin(async move {
            let mut items = self
                .items
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.fail_at == Some(items.len()) {
                return Err(SpendQueueError::ZeroCapacity);
            }
            items.push(update);
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

pub(super) fn enqueue_value(step: &Step) -> Result<Value, Box<HelperError>> {
    let queue = RecordingQueue::new(step.fail_at);
    let cost = spend_amount(step.response_cost.as_ref())?;
    let database: Option<&dyn EntitySpendQueue> = if step.database.unwrap_or(true) {
        Some(&queue)
    } else {
        None
    };
    let deployments = step
        .deployments
        .as_ref()
        .map(|deployments| CaseDeployments {
            deployments: deployments.clone(),
        });
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(failure)?;
    let raised = match step.entity.as_deref().unwrap_or_default() {
        "team" => runtime
            .block_on(update_owner_db(
                database,
                cost,
                step.team_id.as_deref(),
                step.user_id.as_deref(),
                OwnerKind::Team,
            ))
            .is_err(),
        "org" => runtime
            .block_on(update_owner_db(
                database,
                cost,
                step.org_id.as_deref(),
                step.user_id.as_deref(),
                OwnerKind::Organization,
            ))
            .is_err(),
        "project" => runtime
            .block_on(update_row_db(
                database,
                cost,
                step.project_id.as_deref(),
                RowKind::Project,
            ))
            .is_err(),
        "agent" => runtime
            .block_on(update_row_db(
                database,
                cost,
                step.agent_id.as_deref(),
                RowKind::Agent,
            ))
            .is_err(),
        "tag" => {
            let tags = step.request_tags.as_deref().map(RequestTags::Serialized);
            runtime
                .block_on(update_tag_db(database, cost, tags))
                .is_err()
        }
        "model_access_group" => {
            let attributed = step.attributed.clone().unwrap_or_default();
            runtime.block_on(update_model_access_group_db(
                database,
                cost,
                &attributed,
                step.served_model_id.as_deref(),
                deployments
                    .as_ref()
                    .map(|deployments| deployments as &dyn ServedDeployments),
            ));
            false
        }
        other => return Err(failure(format!("unknown spend_entities entity: {other}"))),
    };
    let queued = queue.items().iter().map(queued_view).collect();
    serde_json::to_value(EnqueueView { queued, raised }).map_err(failure)
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
        Some(value) => serde_json::from_value::<Decimal>(value.clone())
            .map(|amount| Some(SpendAmount(amount)))
            .map_err(failure),
    }
}
