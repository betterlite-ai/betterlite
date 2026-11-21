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

// How an adapter names the identity its row is queued under, once the source's common base exists.
enum Charged<'call> {
    // The projection's first identity, a present null included: the user row is charged either way.
    First,
    // An identity the caller already holds: the organization id and the payload's end user.
    Named(&'call str),
    // Every identity the projection expands to, each keeping the payload's request id: the tags.
    Every,
}

// The queue, payload and pricing state the six adapters hand to the shared charge.
struct DailyCall<'call, C> {
    queue: Option<&'call dyn DailySpendQueue>,
    payload: &'call DailyPayload,
    pricing: &'call DailyPricing<'call, C>,
    endpoints: &'call dyn CallTypeEndpoints,
}

impl<C: SavingsPricingContext> DailyCall<'_, C> {
    // _common_add_spend_log_transaction_to_daily_transaction, then the identity's own gate and one queue update per charged identity.
    async fn charge(
        &self,
        entity: DailySpendEntity,
        charged: Charged<'_>,
    ) -> Result<(), DailyRecordError> {
        let Some(sink) = self.queue else {
            return Ok(());
        };
        let Some(projection) =
            common_daily_transaction(self.payload, entity, self.pricing, self.endpoints).await?
        else {
            return Ok(());
        };
        match charged {
            Charged::First => {
                let identity = projection.identities.first().cloned().flatten();
                let update = entity_update(&projection, entity, identity.as_deref(), None);
                enqueue_update(sink, entity, update).await
            }
            Charged::Named(identity) => {
                let update = entity_update(&projection, entity, Some(identity), None);
                enqueue_update(sink, entity, update).await
            }
            Charged::Every => {
                let request_id = self.payload.request_id.as_deref();
                for identity in projection.identities.iter().flatten() {
                    let update = entity_update(&projection, entity, Some(identity), request_id);
                    enqueue_update(sink, entity, update).await?;
                }
                Ok(())
            }
        }
    }
}

// LiteLLM writes each daily adapter out on its own; one table keeps the queue plumbing here, and the two adapters that rewrite the payload keep that rewrite in their own arm.
#[rustfmt::skip]
macro_rules! daily_callers {
    ($name:ident, $entity:expr, $charged:expr) => {
        pub async fn $name<C>(
            queue: Option<&dyn DailySpendQueue>,
            payload: &DailyPayload,
            pricing: &DailyPricing<'_, C>,
            endpoints: &dyn CallTypeEndpoints,
        ) -> Result<(), DailyRecordError>
        where
            C: SavingsPricingContext,
        {
            let call = DailyCall { queue, payload, pricing, endpoints };
            call.charge($entity, $charged).await
        }
    };
    (end_user $name:ident) => {
        pub async fn $name<C>(
            queue: Option<&dyn DailySpendQueue>,
            payload: &DailyPayload,
            pricing: &DailyPricing<'_, C>,
            endpoints: &dyn CallTypeEndpoints,
        ) -> Result<(), DailyRecordError>
        where
            C: SavingsPricingContext,
        {
            let Some(end_user_key) = payload.end_user.as_deref().filter(|value| !value.is_empty()) else {
                return Ok(());
            };
            let mut normalized = payload.clone();
            normalized.end_user_id = Some(end_user_key.to_owned());
            let call = DailyCall { queue, payload: &normalized, pricing, endpoints };
            call.charge(DailySpendEntity::EndUser, Charged::Named(end_user_key)).await
        }
    };
    (org $name:ident) => {
        pub async fn $name<C>(
            queue: Option<&dyn DailySpendQueue>,
            payload: &DailyPayload,
            org_id: Option<&str>,
            pricing: &DailyPricing<'_, C>,
            endpoints: &dyn CallTypeEndpoints,
        ) -> Result<(), DailyRecordError>
        where
            C: SavingsPricingContext,
        {
            let Some(org_key) = org_id else {
                return Ok(());
            };
            let mut normalized = payload.clone();
            normalized.organization_id = Some(org_key.to_owned());
            let call = DailyCall { queue, payload: &normalized, pricing, endpoints };
            call.charge(DailySpendEntity::Org, Charged::Named(org_key)).await
        }
    };
}

// add_spend_log_transaction_to_daily_user_transaction: a present null user still rolls up, under the None text of the queue key.
daily_callers!(
    add_daily_user_transaction,
    DailySpendEntity::User,
    Charged::First
);

// add_spend_log_transaction_to_daily_team_transaction: a null team_id skips the row, as its own id gate already does.
daily_callers!(
    add_daily_team_transaction,
    DailySpendEntity::Team,
    Charged::First
);

// add_spend_log_transaction_to_daily_org_transaction: the explicit org id is the one the row is charged, even when the payload held another.
daily_callers!(org add_daily_org_transaction);

// add_spend_log_transaction_to_daily_end_user_transaction: a nonempty end_user becomes end_user_id, and an empty one skips the row.
daily_callers!(end_user add_daily_end_user_transaction);

// add_spend_log_transaction_to_daily_agent_transaction: only a null agent_id skips the row, so an empty one still charges its agent.
daily_callers!(
    add_daily_agent_transaction,
    DailySpendEntity::Agent,
    Charged::First
);

// add_spend_log_transaction_to_daily_tag_transaction: one row per deduplicated tag, each keeping the request id, and the source's own null check after the ids gate can no longer fire.
daily_callers!(
    add_daily_tag_transaction,
    DailySpendEntity::Tag,
    Charged::Every
);

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
