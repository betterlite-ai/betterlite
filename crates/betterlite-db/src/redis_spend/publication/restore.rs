use std::collections::BTreeMap;
use std::sync::PoisonError;

use futures_util::future::FutureExt as _;

use crate::redis_spend::keys::SpendBufferQueue;
use crate::spend_queues::daily::{DailySpendUpdate, DailySpendUpdateQueue};
use crate::spend_queues::entity::{
    EntityType, SpendAmount, SpendUpdateQueue, SpendUpdateQueueItem, SpendUpdateTransactions,
};
use crate::window_queue::WindowSpendTransaction;
use crate::window_queue::queue::WindowSpendUpdateQueue;

use super::{DrainedCohorts, RedisSpendPublisher, SpendPublishError, SpendUpdateQueues};

// _restore_spend_updates_to_in_memory_queues: entity increments go back through add_update, and a daily or window queue that is already full stages its cohort for the next tick instead of waiting on the flusher that drained it.
pub(super) async fn restore_cohorts(
    publisher: &RedisSpendPublisher,
    queues: SpendUpdateQueues<'_>,
    cohorts: DrainedCohorts,
) -> Result<(), SpendPublishError> {
    let DrainedCohorts {
        entity,
        daily_user,
        daily_team,
        daily_org,
        daily_end_user,
        daily_agent,
        window,
    } = cohorts;
    restore_entity(&entity, queues.entity).await?;
    let dailies = [
        (SpendBufferQueue::DailyUser, queues.daily_user, daily_user),
        (SpendBufferQueue::DailyTeam, queues.daily_team, daily_team),
        (SpendBufferQueue::DailyOrg, queues.daily_org, daily_org),
        (
            SpendBufferQueue::DailyEndUser,
            queues.daily_end_user,
            daily_end_user,
        ),
        (
            SpendBufferQueue::DailyAgent,
            queues.daily_agent,
            daily_agent,
        ),
    ];
    for (queue_name, queue, update) in dailies {
        restore_daily(publisher, queue_name, queue, update)?;
    }
    if let Some(window_queue) = queues.window {
        restore_window(publisher, window_queue, window)?;
    }
    Ok(())
}

// The entity buckets in the order the source re-enqueues them.
const fn entity_buckets(
    transactions: &SpendUpdateTransactions,
) -> [(EntityType, &BTreeMap<String, SpendAmount>); 11] {
    [
        (EntityType::User, &transactions.user_list_transactions),
        (
            EntityType::EndUser,
            &transactions.end_user_list_transactions,
        ),
        (EntityType::Key, &transactions.key_list_transactions),
        (EntityType::Team, &transactions.team_list_transactions),
        (
            EntityType::TeamMember,
            &transactions.team_member_list_transactions,
        ),
        (
            EntityType::Organization,
            &transactions.org_list_transactions,
        ),
        (
            EntityType::OrganizationMember,
            &transactions.org_member_list_transactions,
        ),
        (EntityType::Project, &transactions.project_list_transactions),
        (EntityType::Tag, &transactions.tag_list_transactions),
        (EntityType::Agent, &transactions.agent_list_transactions),
        (
            EntityType::ModelAccessGroup,
            &transactions.model_access_group_spend,
        ),
    ]
}

// One SpendUpdateQueueItem per entity id, exactly the increment the drain aggregated away.
async fn restore_entity(
    transactions: &SpendUpdateTransactions,
    queue: &SpendUpdateQueue,
) -> Result<(), SpendPublishError> {
    for (entity_type, increments) in entity_buckets(transactions) {
        for (entity_id, cost) in increments {
            queue
                .add_update(SpendUpdateQueueItem {
                    entity_type: Some(entity_type.clone()),
                    entity_id: Some(entity_id.clone()),
                    response_cost: Some(*cost),
                })
                .await?;
        }
    }
    Ok(())
}

// The source's `await queue.put(...)`: one poll for room, and a full queue keeps the cohort in the publisher instead of blocking the scheduler.
fn restore_daily(
    publisher: &RedisSpendPublisher,
    queue_name: SpendBufferQueue,
    queue: &DailySpendUpdateQueue,
    update: DailySpendUpdate,
) -> Result<(), SpendPublishError> {
    if update.is_empty() {
        return Ok(());
    }
    let Some(outcome) = queue.add_update(update.clone()).now_or_never() else {
        let mut staging = publisher
            .staging
            .inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        staging.daily.entry(queue_name).or_default().push(update);
        drop(staging);
        return Ok(());
    };
    outcome.map_err(SpendPublishError::from)
}

fn restore_window(
    publisher: &RedisSpendPublisher,
    queue: &WindowSpendUpdateQueue,
    transactions: Vec<WindowSpendTransaction>,
) -> Result<(), SpendPublishError> {
    for transaction in transactions {
        let Some(outcome) = queue.add_update(transaction.clone()).now_or_never() else {
            let mut staging = publisher
                .staging
                .inner
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            staging.window.push(transaction);
            drop(staging);
            continue;
        };
        outcome?;
    }
    Ok(())
}
