use std::error::Error as StdError;

use betterlite_models::savings_pricing::context::SavingsPricingContext;
use rust_decimal::Decimal;

use super::isolation::isolate;
use super::request_tags;
use super::{SpendAccounting, SpendAccountingEnvironment};
use crate::daily_records::projection::DailyPayload;
use crate::spend_attribution::enqueue::{
    EntitySpendQueue, OwnerKind, RowKind, update_model_access_group_db, update_owner_db,
    update_row_db, update_tag_db,
};
use crate::spend_log_storage::batch_claim::BatchClaimTable;
use crate::spend_queues::SpendQueueError;
use crate::spend_queues::entity::{EntityType, SpendAmount, SpendUpdateQueueItem};
use crate::spend_writer::{DbSpendUpdateWriter, SpendWriterFailure, SpendWriterOperation};

// _update_user_db and _update_key_db: a user failure is logged and swallowed, a key failure goes to the caller's own catch.
pub(super) async fn identity_updates<Table>(
    writer: &DbSpendUpdateWriter<'_, Table>,
    work: &SpendAccounting,
) where
    Table: BatchClaimTable + ?Sized,
{
    let Some(queue) = writer.queues.entity else {
        return;
    };
    if let Err(source) = enqueue_user_rows(writer, queue, work).await {
        report_accounting_failure(writer, SpendWriterOperation::UserEnqueue, work, &source);
    }
    let Some(key) = work.hashed_token.as_deref() else {
        return;
    };
    if let Err(source) = queue
        .enqueue(SpendUpdateQueueItem {
            entity_type: Some(EntityType::Key),
            entity_id: Some(key.to_owned()),
            response_cost: work.response_cost,
        })
        .await
    {
        report_accounting_failure(writer, SpendWriterOperation::KeyEnqueue, work, &source);
        tracing::debug!(
            error = %source,
            "_batch_database_updates: the key enqueue failed"
        );
    }
}

// _update_team_db, _update_org_db, _update_project_db and _update_tag_db, each isolated so one failure leaves the rest running.
pub(super) async fn owner_row_updates<Table>(
    writer: &DbSpendUpdateWriter<'_, Table>,
    work: &SpendAccounting,
    daily: Option<&DailyPayload>,
) where
    Table: BatchClaimTable + ?Sized,
{
    let queue = writer.queues.entity;
    let cost = work.response_cost;
    let user = work.user_id.as_deref();
    let org = work.org_id.as_deref();
    isolate(
        "team",
        update_owner_db(queue, cost, work.team_id.as_deref(), user, OwnerKind::Team).await,
    );
    isolate(
        "organization",
        update_owner_db(queue, cost, org, user, OwnerKind::Organization).await,
    );
    isolate(
        "project",
        update_row_db(queue, cost, work.project_id.as_deref(), RowKind::Project).await,
    );
    isolate(
        "tag",
        update_tag_db(queue, cost, daily.and_then(request_tags)).await,
    );
}

// update_model_access_group_db has no isolation of its own; the agent row after it is isolated.
pub(super) async fn attribute_updates<Table, C>(
    writer: &DbSpendUpdateWriter<'_, Table>,
    work: &SpendAccounting,
    daily: Option<&DailyPayload>,
    environment: &SpendAccountingEnvironment<'_, C>,
) where
    Table: BatchClaimTable + ?Sized,
    C: SavingsPricingContext,
{
    update_model_access_group_db(
        writer.queues.entity,
        work.response_cost,
        &work.model_access_groups,
        daily.and_then(|payload| payload.model_id.as_deref()),
        environment.deployments,
    )
    .await;
    isolate(
        "agent",
        update_row_db(
            writer.queues.entity,
            work.response_cost,
            daily.and_then(|payload| payload.agent_id.as_deref()),
            RowKind::Agent,
        )
        .await,
    );
}

async fn enqueue_user_rows<Table>(
    writer: &DbSpendUpdateWriter<'_, Table>,
    queue: &dyn EntitySpendQueue,
    work: &SpendAccounting,
) -> Result<(), SpendQueueError>
where
    Table: BatchClaimTable + ?Sized,
{
    if let Some(user) = work.user_id.as_deref() {
        enqueue_user(queue, user, work.response_cost).await?;
    }
    if writer.settings.max_budget > Decimal::ZERO {
        enqueue_user(
            queue,
            &writer.settings.proxy_budget_name,
            work.response_cost,
        )
        .await?;
    }
    if let Some(end_user) = work.end_user_id.as_deref() {
        queue
            .enqueue(SpendUpdateQueueItem {
                entity_type: Some(EntityType::EndUser),
                entity_id: Some(end_user.to_owned()),
                response_cost: work.response_cost,
            })
            .await?;
    }
    Ok(())
}

fn report_accounting_failure<Table>(
    writer: &DbSpendUpdateWriter<'_, Table>,
    operation: SpendWriterOperation,
    work: &SpendAccounting,
    error: &(dyn StdError + Send + Sync),
) where
    Table: BatchClaimTable + ?Sized,
{
    writer.failures.report(SpendWriterFailure {
        operation,
        response_cost: work.response_cost,
        token: work.hashed_token.as_deref(),
        user_id: work.user_id.as_deref(),
        team_id: work.team_id.as_deref(),
        org_id: work.org_id.as_deref(),
        end_user_id: work.end_user_id.as_deref(),
        error,
    });
}

async fn enqueue_user(
    queue: &dyn EntitySpendQueue,
    user: &str,
    response_cost: Option<SpendAmount>,
) -> Result<(), SpendQueueError> {
    queue
        .enqueue(SpendUpdateQueueItem {
            entity_type: Some(EntityType::User),
            entity_id: Some(user.to_owned()),
            response_cost,
        })
        .await
}
