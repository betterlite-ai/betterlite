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

// _update_team_db: the team is queued first, then its member row; a member failure is logged and swallowed, a team failure is logged and rethrown.
pub async fn update_team_db(
    queue: Option<&dyn EntitySpendQueue>,
    response_cost: Option<SpendAmount>,
    team_id: Option<&str>,
    user_id: Option<&str>,
) -> Result<(), SpendAttributionError> {
    let (Some(queue), Some(team_id)) = (queue, team_id) else {
        tracing::debug!("No team id or no database, not tracking spend for team");
        return Ok(());
    };
    if let Err(source) = queue
        .enqueue(item(EntityType::Team, team_id, response_cost))
        .await
    {
        tracing::error!(team_id, error = %source, "Failed to enqueue team spend update");
        return Err(SpendAttributionError::Enqueue {
            entity: "team",
            source,
        });
    }
    let Some(user_id) = user_id else {
        return Ok(());
    };
    // key is "team_id::<value>::user_id::<value>", with neither value escaped
    let member_key = format!("team_id::{team_id}::user_id::{user_id}");
    if let Err(source) = queue
        .enqueue(item(EntityType::TeamMember, &member_key, response_cost))
        .await
    {
        tracing::error!(
            team_id,
            user_id,
            error = %source,
            "Failed to enqueue team member spend update"
        );
    }
    Ok(())
}

// _update_org_db: the org row then its member row, and either enqueue failure is logged and passed up.
pub async fn update_org_db(
    queue: Option<&dyn EntitySpendQueue>,
    response_cost: Option<SpendAmount>,
    org_id: Option<&str>,
    user_id: Option<&str>,
) -> Result<(), SpendAttributionError> {
    let (Some(queue), Some(org_id)) = (queue, org_id) else {
        tracing::debug!("No org id or no database, not tracking spend for org");
        return Ok(());
    };
    if let Err(source) = queue
        .enqueue(item(EntityType::Organization, org_id, response_cost))
        .await
    {
        tracing::error!(org_id, error = %source, "Failed to enqueue org spend update");
        return Err(SpendAttributionError::Enqueue {
            entity: "organization",
            source,
        });
    }
    let Some(user_id) = user_id else {
        return Ok(());
    };
    let member_key = org_member_transaction_key(org_id, user_id);
    if let Err(source) = queue
        .enqueue(item(
            EntityType::OrganizationMember,
            &member_key,
            response_cost,
        ))
        .await
    {
        tracing::error!(org_id, error = %source, "Failed to enqueue org member spend update");
        return Err(SpendAttributionError::Enqueue {
            entity: "organization member",
            source,
        });
    }
    Ok(())
}

// _update_project_db: no id or no database returns before the try, so neither path logs.
pub async fn update_project_db(
    queue: Option<&dyn EntitySpendQueue>,
    response_cost: Option<SpendAmount>,
    project_id: Option<&str>,
) -> Result<(), SpendAttributionError> {
    let (Some(queue), Some(project_id)) = (queue, project_id) else {
        return Ok(());
    };
    if let Err(source) = queue
        .enqueue(item(EntityType::Project, project_id, response_cost))
        .await
    {
        tracing::error!(project_id, error = %source, "Failed to enqueue project spend update");
        return Err(SpendAttributionError::Enqueue {
            entity: "project",
            source,
        });
    }
    Ok(())
}

// _update_agent_db: the same silent skip as the project row, then a logged and rethrown enqueue failure.
pub async fn update_agent_db(
    queue: Option<&dyn EntitySpendQueue>,
    response_cost: Option<SpendAmount>,
    agent_id: Option<&str>,
) -> Result<(), SpendAttributionError> {
    let (Some(queue), Some(agent_id)) = (queue, agent_id) else {
        return Ok(());
    };
    if let Err(source) = queue
        .enqueue(item(EntityType::Agent, agent_id, response_cost))
        .await
    {
        tracing::error!(agent_id, error = %source, "Failed to enqueue agent spend update");
        return Err(SpendAttributionError::Enqueue {
            entity: "agent",
            source,
        });
    }
    Ok(())
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
        if let Err(source) = queue
            .enqueue(item(EntityType::Tag, tag, response_cost))
            .await
        {
            tracing::error!(tag, error = %source, "Failed to enqueue tag spend update");
            return Err(SpendAttributionError::Enqueue {
                entity: "tag",
                source,
            });
        }
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
        if let Err(source) = queue
            .enqueue(item(EntityType::ModelAccessGroup, &group, response_cost))
            .await
        {
            tracing::error!(
                model_access_group = %group,
                error = %source,
                "Failed to enqueue model access group spend update"
            );
            return;
        }
    }
}

#[path = "enqueue/tests.rs"]
mod tests;
