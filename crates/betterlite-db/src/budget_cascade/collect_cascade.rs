use rust_decimal::Decimal;
use std::collections::BTreeMap;

use super::fetch_failure::fetched_or_empty;
use super::linked_rows::{
    BudgetLinkedRow, KeyRow, LinkedKeys, LinkedRows, ModelAccessGroupRow, OrganizationRow,
    ProjectRow, TagRow, TeamMembershipRow,
};
use super::rollover_carry::rollover_caps;
use super::{BudgetCascade, BudgetReset, CounterReset};
use crate::budget_permission_rows::LiteLLMBudgetTableFull;
use crate::budget_reset_policy::entity_reset::ResetSchedule;
use crate::budget_reset_policy::rollover::{RolloverPolicy, row_carried_spend};

struct FetchedRows {
    team_memberships: Vec<TeamMembershipRow>,
    keys: Vec<KeyRow>,
    organizations: Vec<OrganizationRow>,
    tags: Vec<TagRow>,
    model_access_groups: Vec<ModelAccessGroupRow>,
    projects: Vec<ProjectRow>,
}

fn extend_counter_resets<Row>(
    rows: &[Row],
    caps: &BTreeMap<String, Decimal>,
    counter_resets: &mut Vec<CounterReset>,
) where
    Row: BudgetLinkedRow + LinkedKeys,
{
    for row in rows {
        counter_resets.push(CounterReset {
            counter_key: row.counter_key(),
            carried_spend: row_carried_spend(row.spend(), row.budget_id(), caps),
        });
    }
}

fn extend_cache_keys<Row>(rows: &[Row], cache_keys: &mut Vec<String>)
where
    Row: LinkedKeys,
{
    for row in rows {
        cache_keys.extend(row.cache_keys());
    }
}

impl FetchedRows {
    fn counter_resets(&self, caps: &BTreeMap<String, Decimal>) -> Vec<CounterReset> {
        let mut counter_resets = Vec::new();
        extend_counter_resets(&self.team_memberships, caps, &mut counter_resets);
        extend_counter_resets(&self.keys, caps, &mut counter_resets);
        extend_counter_resets(&self.organizations, caps, &mut counter_resets);
        extend_counter_resets(&self.tags, caps, &mut counter_resets);
        extend_counter_resets(&self.model_access_groups, caps, &mut counter_resets);
        extend_counter_resets(&self.projects, caps, &mut counter_resets);
        counter_resets
    }

    fn cache_keys(&self) -> Vec<String> {
        let mut cache_keys = Vec::new();
        extend_cache_keys(&self.team_memberships, &mut cache_keys);
        extend_cache_keys(&self.keys, &mut cache_keys);
        extend_cache_keys(&self.organizations, &mut cache_keys);
        extend_cache_keys(&self.tags, &mut cache_keys);
        extend_cache_keys(&self.model_access_groups, &mut cache_keys);
        extend_cache_keys(&self.projects, &mut cache_keys);
        cache_keys
    }
}

async fn fetch_linked_rows<Source>(source: &Source, budget_ids: &[String]) -> FetchedRows
where
    Source: LinkedRows + Sync,
{
    FetchedRows {
        team_memberships: fetched_or_empty("team memberships", || {
            source.team_memberships(budget_ids)
        })
        .await,
        keys: fetched_or_empty("keys", || source.keys(budget_ids)).await,
        organizations: fetched_or_empty("orgs", || source.organizations(budget_ids)).await,
        tags: fetched_or_empty("tags", || source.tags(budget_ids)).await,
        model_access_groups: fetched_or_empty("model access groups", || {
            source.model_access_groups(budget_ids)
        })
        .await,
        projects: fetched_or_empty("projects", || source.projects(budget_ids)).await,
    }
}

// Every tier with a duration gets its next window computed through B2317's reset settings; a tier without one is not a timed tier.
fn compute_budget_resets<Schedule>(
    budgets_to_reset: &[LiteLLMBudgetTableFull],
    schedule: &Schedule,
) -> Result<Vec<BudgetReset>, Schedule::Failure>
where
    Schedule: ResetSchedule,
{
    let mut resets = Vec::new();
    for budget in budgets_to_reset {
        let Some(budget_id) = budget.budget.budget_id.as_deref() else {
            continue;
        };
        let Some(duration) = budget.budget.budget_duration.as_deref() else {
            continue;
        };
        resets.push(BudgetReset {
            budget_id: budget_id.to_owned(),
            reset_at: schedule.next_reset_at(duration)?,
        });
    }
    Ok(resets)
}

pub async fn collect_budget_cascade<Source, Schedule>(
    source: &Source,
    budgets_to_reset: &[LiteLLMBudgetTableFull],
    policy: RolloverPolicy,
    schedule: &Schedule,
) -> Result<BudgetCascade, Schedule::Failure>
where
    Source: LinkedRows + Sync,
    Schedule: ResetSchedule + Sync,
{
    let budget_ids: Vec<String> = budgets_to_reset
        .iter()
        .filter_map(|budget| budget.budget.budget_id.clone())
        .collect();
    if budget_ids.is_empty() {
        return Ok(BudgetCascade::default());
    }
    let rows = fetch_linked_rows(source, &budget_ids).await;
    let caps = rollover_caps(budgets_to_reset, policy);
    Ok(BudgetCascade {
        budgets: budgets_to_reset.to_vec(),
        budget_ids,
        budget_resets: compute_budget_resets(budgets_to_reset, schedule)?,
        counter_resets: rows.counter_resets(&caps),
        cache_keys: rows.cache_keys(),
        rollover_caps: caps,
    })
}

mod tests;
