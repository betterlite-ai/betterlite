use std::error::Error as StdError;

use rust_decimal::Decimal;

use super::isolation::isolated;
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

impl<Table> DbSpendUpdateWriter<'_, Table>
where
    Table: BatchClaimTable + ?Sized,
{
    // The entity rows in source order: the user block first, then key, team, organization, project, tag, access groups and agent.
    pub(super) async fn entity_updates<C>(
        &self,
        work: &SpendAccounting,
        daily: Option<&DailyPayload>,
        environment: &SpendAccountingEnvironment<'_, C>,
    ) {
        self.update_user_db(work).await;
        if let Err(source) = self.update_key_db(work).await {
            self.report_accounting_failure(SpendWriterOperation::KeyEnqueue, work, &source);
            tracing::debug!(
                error = %source,
                "_batch_database_updates: the key enqueue failed"
            );
        }
        isolated!(
            "team",
            update_owner_db(
                self.queues.entity,
                work.response_cost,
                work.team_id.as_deref(),
                work.user_id.as_deref(),
                OwnerKind::Team,
            )
        );
        isolated!(
            "organization",
            update_owner_db(
                self.queues.entity,
                work.response_cost,
                work.org_id.as_deref(),
                work.user_id.as_deref(),
                OwnerKind::Organization,
            )
        );
        isolated!(
            "project",
            update_row_db(
                self.queues.entity,
                work.response_cost,
                work.project_id.as_deref(),
                RowKind::Project,
            )
        );
        isolated!(
            "tag",
            update_tag_db(
                self.queues.entity,
                work.response_cost,
                daily.and_then(request_tags),
            )
        );
        update_model_access_group_db(
            self.queues.entity,
            work.response_cost,
            &work.model_access_groups,
            daily.and_then(|payload| payload.model_id.as_deref()),
            environment.deployments,
        )
        .await;
        isolated!(
            "agent",
            update_row_db(
                self.queues.entity,
                work.response_cost,
                daily.and_then(|payload| payload.agent_id.as_deref()),
                RowKind::Agent,
            )
        );
    }

    // _update_user_db: the absent ids are skipped, a positive budget adds the proxy identity, and a failure is logged and swallowed.
    async fn update_user_db(&self, work: &SpendAccounting) {
        let Some(queue) = self.queues.entity else {
            return;
        };
        if let Err(source) = self.enqueue_user_rows(queue, work).await {
            self.report_accounting_failure(SpendWriterOperation::UserEnqueue, work, &source);
        }
    }

    async fn enqueue_user_rows(
        &self,
        queue: &dyn EntitySpendQueue,
        work: &SpendAccounting,
    ) -> Result<(), SpendQueueError> {
        if let Some(user) = work.user_id.as_deref() {
            enqueue_user(queue, user, work.response_cost).await?;
        }
        if self.settings.max_budget > Decimal::ZERO {
            enqueue_user(queue, &self.settings.proxy_budget_name, work.response_cost).await?;
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

    // _update_key_db: a key is charged whenever one was hashed; its failure is logged and raised to the caller's own catch.
    async fn update_key_db(&self, work: &SpendAccounting) -> Result<(), SpendQueueError> {
        let (Some(queue), Some(key)) = (self.queues.entity, work.hashed_token.as_deref()) else {
            return Ok(());
        };
        queue
            .enqueue(SpendUpdateQueueItem {
                entity_type: Some(EntityType::Key),
                entity_id: Some(key.to_owned()),
                response_cost: work.response_cost,
            })
            .await
    }

    fn report_accounting_failure(
        &self,
        operation: SpendWriterOperation,
        work: &SpendAccounting,
        error: &(dyn StdError + Send + Sync),
    ) {
        self.failures.report(SpendWriterFailure {
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
