use std::future::{Future, ready};
use std::pin::Pin;

use betterlite_models::savings_pricing::context::SavingsPricingContext;

use super::DbSpendUpdateWriter;
use crate::daily_records::callers::{
    add_daily_agent_transaction, add_daily_end_user_transaction, add_daily_org_transaction,
    add_daily_tag_transaction, add_daily_team_transaction, add_daily_user_transaction,
};
use crate::daily_records::projection::{CallTypeEndpoints, DailyPayload, DailyPricing};
use crate::daily_spend_sql::tags::TagsPayload;
use crate::redis_spend::log_row::SpendLogRow;
use crate::request_usage::enqueue::enqueue_model_usage;
use crate::spend_attribution::access_groups::ServedDeployments;
use crate::spend_attribution::tags::RequestTags;
use crate::spend_log_storage::batch_claim::BatchClaimTable;
use crate::spend_queues::entity::SpendAmount;

use self::entities::{attribute_updates, identity_updates, owner_updates, row_updates};
use self::isolation::isolated;

mod entities;
mod isolation;
mod tests;

// The owned work item one request hands to the accounting worker: the ids the sibling helpers charge and the log payload they project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpendAccounting {
    pub response_cost: Option<SpendAmount>,
    pub user_id: Option<String>,
    pub hashed_token: Option<String>,
    pub team_id: Option<String>,
    pub org_id: Option<String>,
    pub project_id: Option<String>,
    pub end_user_id: Option<String>,
    pub model_access_groups: Vec<String>,
    pub payload: SpendLogRow,
}

#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("the spend accounting worker is no longer accepting work")]
pub struct SpendAccountingClosed;

pub type AdmissionFuture<'call> =
    Pin<Box<dyn Future<Output = Result<(), SpendAccountingClosed>> + Send + 'call>>;

// The bounded handoff between update_database and the accounting worker; a full queue waits instead of dropping a charge.
pub trait SpendAccountingAdmission: Send + Sync {
    fn admit(&self, work: SpendAccounting) -> AdmissionFuture<'_>;
}

// The accounting worker itself writes work that was already admitted, so its writer never admits a second time.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSpendAccountingAdmission;

impl SpendAccountingAdmission for NoSpendAccountingAdmission {
    fn admit(&self, _work: SpendAccounting) -> AdmissionFuture<'_> {
        Box::pin(ready(Err(SpendAccountingClosed)))
    }
}

// The pricing, call-type and deployment lookups the daily builders read, supplied by the runtime that owns them.
pub struct SpendAccountingEnvironment<'env, C> {
    pub pricing: &'env DailyPricing<'env, C>,
    pub endpoints: &'env dyn CallTypeEndpoints,
    pub deployments: Option<&'env dyn ServedDeployments>,
}

// _batch_database_updates returns to charge the payload in its own task: the payload is read again here, never borrowed from the request.
pub async fn batch_database_updates<Table, C>(
    writer: &DbSpendUpdateWriter<'_, Table>,
    work: &SpendAccounting,
    environment: &SpendAccountingEnvironment<'_, C>,
) where
    Table: BatchClaimTable + ?Sized,
    C: SavingsPricingContext,
{
    let payload = match serde_json::to_vec(&work.payload) {
        Ok(serialized) => serialized,
        Err(error) => {
            tracing::debug!(
                error = %error,
                "_batch_database_updates: the spend log row cannot be written back as JSON"
            );
            return;
        }
    };
    let daily: Option<DailyPayload> = match serde_json::from_slice(&payload) {
        Ok(parsed) => Some(parsed),
        Err(error) => {
            tracing::debug!(
                error = %error,
                "_batch_database_updates: the spend log row has no daily projection"
            );
            None
        }
    };
    identity_updates(writer, work).await;
    owner_updates(writer, work).await;
    row_updates(writer, work, daily.as_ref()).await;
    attribute_updates(writer, work, daily.as_ref(), environment).await;
    if let Some(projected) = daily.as_ref() {
        daily_user_updates(writer, projected, environment).await;
        daily_owner_updates(writer, work, projected, environment).await;
    }
    if let Some(queue) = writer.queues.model_usage {
        enqueue_model_usage(queue, &payload).await;
    }
}

// The user, end user and agent rows, each isolated from the others.
async fn daily_user_updates<Table, C>(
    writer: &DbSpendUpdateWriter<'_, Table>,
    payload: &DailyPayload,
    environment: &SpendAccountingEnvironment<'_, C>,
) where
    Table: BatchClaimTable + ?Sized,
    C: SavingsPricingContext,
{
    isolated!(
        "daily user",
        add_daily_user_transaction(
            writer.queues.daily_user,
            payload,
            environment.pricing,
            environment.endpoints,
        )
        .await
    );
    isolated!(
        "daily end user",
        add_daily_end_user_transaction(
            writer.queues.daily_end_user,
            payload,
            environment.pricing,
            environment.endpoints,
        )
        .await
    );
    isolated!(
        "daily agent",
        add_daily_agent_transaction(
            writer.queues.daily_agent,
            payload,
            environment.pricing,
            environment.endpoints,
        )
        .await
    );
}

// The team, organization and tag rows, with the organization charged under the request's own id.
async fn daily_owner_updates<Table, C>(
    writer: &DbSpendUpdateWriter<'_, Table>,
    work: &SpendAccounting,
    payload: &DailyPayload,
    environment: &SpendAccountingEnvironment<'_, C>,
) where
    Table: BatchClaimTable + ?Sized,
    C: SavingsPricingContext,
{
    isolated!(
        "daily team",
        add_daily_team_transaction(
            writer.queues.daily_team,
            payload,
            environment.pricing,
            environment.endpoints,
        )
        .await
    );
    isolated!(
        "daily organization",
        add_daily_org_transaction(
            writer.queues.daily_org,
            payload,
            work.org_id.as_deref(),
            environment.pricing,
            environment.endpoints,
        )
        .await
    );
    isolated!(
        "daily tag",
        add_daily_tag_transaction(
            writer.queues.daily_tag,
            payload,
            environment.pricing,
            environment.endpoints,
        )
        .await
    );
}

// _update_tag_db reads the serialized column through safe_json_loads and an already parsed list as sent.
fn request_tags(payload: &DailyPayload) -> Option<RequestTags<'_>> {
    match payload.request_tags.as_ref()? {
        TagsPayload::Text(text) => Some(RequestTags::Serialized(text)),
        TagsPayload::Array(tags) => Some(RequestTags::Parsed(tags)),
    }
}
