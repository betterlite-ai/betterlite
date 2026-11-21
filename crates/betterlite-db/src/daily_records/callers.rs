use std::future::Future;
use std::pin::Pin;

use betterlite_models::savings_pricing::context::SavingsPricingContext;

use crate::daily_spend_sql::entities::DailySpendEntity;
use crate::spend_queues::SpendQueueError;
use crate::spend_queues::daily::{DailySpendUpdate, DailySpendUpdateQueue};

use super::DailyRecordError;
use super::projection::rows::{entity_spend_row, queue_key};
use super::projection::{
    CallTypeEndpoints, DailyPayload, DailyPricing, DailyProjection, common_daily_transaction,
};

pub type QueueFuture<'queue> =
    Pin<Box<dyn Future<Output = Result<(), SpendQueueError>> + Send + 'queue>>;

// DailySpendUpdateQueue.add_update behind a port: the writer charges whatever queue it was built with, and a test can fail one enqueue on purpose.
pub trait DailySpendQueue: Send + Sync {
    fn enqueue(&self, update: DailySpendUpdate) -> QueueFuture<'_>;
}

impl DailySpendQueue for DailySpendUpdateQueue {
    fn enqueue(&self, update: DailySpendUpdate) -> QueueFuture<'_> {
        Box::pin(self.add_update(update))
    }
}

// add_spend_log_transaction_to_daily_user_transaction: a present null user still rolls up, under the None text of the queue key.
pub async fn add_daily_user_transaction<C>(
    queue: Option<&dyn DailySpendQueue>,
    payload: &DailyPayload,
    pricing: &DailyPricing<'_, C>,
    endpoints: &dyn CallTypeEndpoints,
) -> Result<(), DailyRecordError>
where
    C: SavingsPricingContext,
{
    let Some(sink) = queue else {
        return Ok(());
    };
    let projection =
        common_daily_transaction(payload, DailySpendEntity::User, pricing, endpoints).await?;
    let Some(projection) = projection else {
        return Ok(());
    };
    let identity = projection.identities.first().cloned().flatten();
    let update = entity_update(
        &projection,
        DailySpendEntity::User,
        identity.as_deref(),
        None,
    );
    enqueue_update(sink, DailySpendEntity::User, update).await
}

// add_spend_log_transaction_to_daily_team_transaction: a null team_id skips the row, as its own id gate already does.
pub async fn add_daily_team_transaction<C>(
    queue: Option<&dyn DailySpendQueue>,
    payload: &DailyPayload,
    pricing: &DailyPricing<'_, C>,
    endpoints: &dyn CallTypeEndpoints,
) -> Result<(), DailyRecordError>
where
    C: SavingsPricingContext,
{
    let Some(sink) = queue else {
        return Ok(());
    };
    let projection =
        common_daily_transaction(payload, DailySpendEntity::Team, pricing, endpoints).await?;
    let Some(projection) = projection else {
        return Ok(());
    };
    let Some(identity) = projection.identities.first().cloned().flatten() else {
        return Ok(());
    };
    let update = entity_update(&projection, DailySpendEntity::Team, Some(&identity), None);
    enqueue_update(sink, DailySpendEntity::Team, update).await
}

// add_spend_log_transaction_to_daily_org_transaction: the explicit org id is the one the row is charged, even when the payload held another.
pub async fn add_daily_org_transaction<C>(
    queue: Option<&dyn DailySpendQueue>,
    payload: &DailyPayload,
    org_id: Option<&str>,
    pricing: &DailyPricing<'_, C>,
    endpoints: &dyn CallTypeEndpoints,
) -> Result<(), DailyRecordError>
where
    C: SavingsPricingContext,
{
    let Some(sink) = queue else {
        return Ok(());
    };
    let Some(org_key) = org_id else {
        return Ok(());
    };
    let mut normalized = payload.clone();
    normalized.organization_id = Some(org_key.to_owned());
    let projection =
        common_daily_transaction(&normalized, DailySpendEntity::Org, pricing, endpoints).await?;
    let Some(projection) = projection else {
        return Ok(());
    };
    let update = entity_update(&projection, DailySpendEntity::Org, Some(org_key), None);
    enqueue_update(sink, DailySpendEntity::Org, update).await
}

// add_spend_log_transaction_to_daily_end_user_transaction: a nonempty end_user becomes end_user_id, and an empty one skips the row.
pub async fn add_daily_end_user_transaction<C>(
    queue: Option<&dyn DailySpendQueue>,
    payload: &DailyPayload,
    pricing: &DailyPricing<'_, C>,
    endpoints: &dyn CallTypeEndpoints,
) -> Result<(), DailyRecordError>
where
    C: SavingsPricingContext,
{
    let Some(sink) = queue else {
        return Ok(());
    };
    let Some(end_user_key) = payload
        .end_user
        .as_deref()
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    let mut normalized = payload.clone();
    normalized.end_user_id = Some(end_user_key.to_owned());
    let projection =
        common_daily_transaction(&normalized, DailySpendEntity::EndUser, pricing, endpoints)
            .await?;
    let Some(projection) = projection else {
        return Ok(());
    };
    let update = entity_update(
        &projection,
        DailySpendEntity::EndUser,
        Some(end_user_key),
        None,
    );
    enqueue_update(sink, DailySpendEntity::EndUser, update).await
}

// add_spend_log_transaction_to_daily_agent_transaction: only a null agent_id skips the row, so an empty one still charges its agent.
pub async fn add_daily_agent_transaction<C>(
    queue: Option<&dyn DailySpendQueue>,
    payload: &DailyPayload,
    pricing: &DailyPricing<'_, C>,
    endpoints: &dyn CallTypeEndpoints,
) -> Result<(), DailyRecordError>
where
    C: SavingsPricingContext,
{
    let Some(sink) = queue else {
        return Ok(());
    };
    let Some(agent_key) = payload.agent_id.as_deref() else {
        return Ok(());
    };
    let projection =
        common_daily_transaction(payload, DailySpendEntity::Agent, pricing, endpoints).await?;
    let Some(projection) = projection else {
        return Ok(());
    };
    let update = entity_update(&projection, DailySpendEntity::Agent, Some(agent_key), None);
    enqueue_update(sink, DailySpendEntity::Agent, update).await
}

// add_spend_log_transaction_to_daily_tag_transaction: one row per deduplicated tag, each keeping the request id, and the source's own null check after the ids gate can no longer fire.
pub async fn add_daily_tag_transaction<C>(
    queue: Option<&dyn DailySpendQueue>,
    payload: &DailyPayload,
    pricing: &DailyPricing<'_, C>,
    endpoints: &dyn CallTypeEndpoints,
) -> Result<(), DailyRecordError>
where
    C: SavingsPricingContext,
{
    let Some(sink) = queue else {
        return Ok(());
    };
    let projection =
        common_daily_transaction(payload, DailySpendEntity::Tag, pricing, endpoints).await?;
    let Some(projection) = projection else {
        return Ok(());
    };
    for tag in projection.identities.iter().flatten() {
        let update = entity_update(
            &projection,
            DailySpendEntity::Tag,
            Some(tag),
            payload.request_id.as_deref(),
        );
        enqueue_update(sink, DailySpendEntity::Tag, update).await?;
    }
    Ok(())
}

fn entity_update(
    projection: &DailyProjection,
    entity: DailySpendEntity,
    identity: Option<&str>,
    request_id: Option<&str>,
) -> DailySpendUpdate {
    let key = queue_key(identity, &projection.base);
    let row = entity_spend_row(&projection.base, entity, identity, request_id);
    DailySpendUpdate::from([(key, row)])
}

async fn enqueue_update(
    queue: &dyn DailySpendQueue,
    entity: DailySpendEntity,
    update: DailySpendUpdate,
) -> Result<(), DailyRecordError> {
    if let Err(source) = queue.enqueue(update).await {
        tracing::error!(
            entity = entity_label(entity),
            error = %source,
            "Failed to enqueue daily spend update"
        );
        return Err(DailyRecordError::Enqueue {
            entity: entity_label(entity),
            source,
        });
    }
    Ok(())
}

fn entity_label(entity: DailySpendEntity) -> &'static str {
    match entity {
        DailySpendEntity::User => "user",
        DailySpendEntity::Team => "team",
        DailySpendEntity::Org => "organization",
        DailySpendEntity::EndUser => "end user",
        DailySpendEntity::Agent => "agent",
        DailySpendEntity::Tag => "tag",
    }
}
