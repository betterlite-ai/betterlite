use std::future::Future;
use std::pin::Pin;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use crate::spend_queues::SpendQueueError;
use crate::spend_queues::entity::{
    EntityType, SpendAmount, SpendUpdateQueue, SpendUpdateQueueItem,
};

use super::SpendAttributionError;
use super::access_groups::{ServedDeployments, debitable_model_access_groups};
use super::tags::{RequestTags, decode_enqueue_tags};

// CPython's urllib.parse.quote with safe='': alphanumerics and _.-~ stay literal, everything else is percent-escaped.
const ORG_MEMBER_QUOTE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');

pub type EnqueueFuture<'a> = Pin<Box<dyn Future<Output = Result<(), SpendQueueError>> + Send + 'a>>;

// SpendUpdateQueue.add_update behind a port: the writer charges whatever queue it was built with, and a test can fail one enqueue on purpose.
pub trait EntitySpendQueue: Send + Sync {
    fn enqueue(&self, update: SpendUpdateQueueItem) -> EnqueueFuture<'_>;
}

impl EntitySpendQueue for SpendUpdateQueue {
    fn enqueue(&self, update: SpendUpdateQueueItem) -> EnqueueFuture<'_> {
        Box::pin(self.add_update(update))
    }
}

// The owner rows _update_team_db and _update_org_db write. The team keeps raw member ids and swallows a member failure; the organization quotes both ids and passes the failure up.
#[derive(Clone, Copy, Debug)]
pub enum OwnerKind {
    Team,
    Organization,
}

// The single rows _update_project_db and _update_agent_db write, which differ only in the entity they charge.
#[derive(Clone, Copy, Debug)]
pub enum RowKind {
    Project,
    Agent,
}

// _org_member_transaction_key: both ids percent-encoded with quote's safe='' so a delimiter inside one stays unreadable as a separator.
#[must_use]
pub fn org_member_transaction_key(org_id: &str, user_id: &str) -> String {
    format!(
        "organization_id::{}::user_id::{}",
        utf8_percent_encode(org_id, ORG_MEMBER_QUOTE_SET),
        utf8_percent_encode(user_id, ORG_MEMBER_QUOTE_SET)
    )
}

fn item(
    entity_type: EntityType,
    entity_id: &str,
    response_cost: Option<SpendAmount>,
) -> SpendUpdateQueueItem {
    SpendUpdateQueueItem {
        entity_type: Some(entity_type),
        entity_id: Some(entity_id.to_owned()),
        response_cost,
    }
}

// One entity row through the queue: a failure is logged with its entity and id and returned for the caller to swallow or pass up.
async fn enqueue_row(
    queue: &dyn EntitySpendQueue,
    entity_type: EntityType,
    entity_id: &str,
    response_cost: Option<SpendAmount>,
    entity: &'static str,
) -> Result<(), SpendAttributionError> {
    if let Err(source) = queue
        .enqueue(item(entity_type, entity_id, response_cost))
        .await
    {
        tracing::error!(
            entity,
            entity_id,
            error = %source,
            "Failed to enqueue spend update"
        );
        return Err(SpendAttributionError::Enqueue { entity, source });
    }
    Ok(())
}

// _update_team_db and _update_org_db: the owner row first, then its member row when a user is known; a team member failure is logged and swallowed, an organization member failure is logged and passed up.
pub async fn update_owner_db(
    queue: Option<&dyn EntitySpendQueue>,
    response_cost: Option<SpendAmount>,
    owner_id: Option<&str>,
    user_id: Option<&str>,
    owner: OwnerKind,
) -> Result<(), SpendAttributionError> {
    let (Some(queue), Some(owner_id)) = (queue, owner_id) else {
        tracing::debug!(
            owner = ?owner,
            "No owner id or no database, not tracking spend"
        );
        return Ok(());
    };
    let (owner_type, owner_label) = match owner {
        OwnerKind::Team => (EntityType::Team, "team"),
        OwnerKind::Organization => (EntityType::Organization, "organization"),
    };
    enqueue_row(queue, owner_type, owner_id, response_cost, owner_label).await?;
    let Some(user_id) = user_id else {
        return Ok(());
    };
    let (member_type, member_label) = match owner {
        OwnerKind::Team => (EntityType::TeamMember, "team member"),
        OwnerKind::Organization => (EntityType::OrganizationMember, "organization member"),
    };
    // key is "team_id::<value>::user_id::<value>" for a team, with neither value escaped
    let member_key = match owner {
        OwnerKind::Team => format!("team_id::{owner_id}::user_id::{user_id}"),
        OwnerKind::Organization => org_member_transaction_key(owner_id, user_id),
    };
    let member = enqueue_row(queue, member_type, &member_key, response_cost, member_label).await;
    match owner {
        OwnerKind::Team => Ok(()),
        OwnerKind::Organization => member,
    }
}

// _update_project_db and _update_agent_db: no id or no database returns before the try, so neither path logs.
pub async fn update_row_db(
    queue: Option<&dyn EntitySpendQueue>,
    response_cost: Option<SpendAmount>,
    row_id: Option<&str>,
    row: RowKind,
) -> Result<(), SpendAttributionError> {
    let (Some(queue), Some(row_id)) = (queue, row_id) else {
        return Ok(());
    };
    let (entity_type, label) = match row {
        RowKind::Project => (EntityType::Project, "project"),
        RowKind::Agent => (EntityType::Agent, "agent"),
    };
    enqueue_row(queue, entity_type, row_id, response_cost, label).await
}

// _update_tag_db: a serialized payload decodes to its iterated names, a parsed list is used as sent, anything else returns silently.
pub async fn update_tag_db(
    queue: Option<&dyn EntitySpendQueue>,
    response_cost: Option<SpendAmount>,
    request_tags: Option<RequestTags<'_>>,
) -> Result<(), SpendAttributionError> {
    let (Some(queue), Some(request_tags)) = (queue, request_tags) else {
        return Ok(());
    };
    let tags = match request_tags {
        RequestTags::Serialized(text) => {
            let tags = match decode_enqueue_tags(text.as_bytes()) {
                Ok(tags) => tags,
                Err(source) => {
                    tracing::error!(error = %source, "Failed to enqueue tag spend update");
                    return Err(SpendAttributionError::Tags { source });
                }
            };
            if tags.is_empty() {
                tracing::debug!("The request tags hold no tag names");
                return Ok(());
            }
            tags
        }
        RequestTags::Parsed(tags) => tags.to_vec(),
    };
    for tag in &tags {
        if tag.is_empty() {
            continue;
        }
        enqueue_row(queue, EntityType::Tag, tag, response_cost, "tag").await?;
    }
    Ok(())
}

// _update_model_access_group_db: every debitable group is charged the full cost, and a failure in the loop is logged and swallowed.
pub async fn update_model_access_group_db(
    queue: Option<&dyn EntitySpendQueue>,
    response_cost: Option<SpendAmount>,
    attributed: &[String],
    served_model_id: Option<&str>,
    deployments: Option<&dyn ServedDeployments>,
) {
    let Some(queue) = queue else {
        return;
    };
    for group in debitable_model_access_groups(attributed, served_model_id, deployments) {
        if enqueue_row(
            queue,
            EntityType::ModelAccessGroup,
            &group,
            response_cost,
            "model access group",
        )
        .await
        .is_err()
        {
            return;
        }
    }
}

#[path = "enqueue/tests.rs"]
mod tests;
