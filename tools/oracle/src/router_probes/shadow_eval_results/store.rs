use std::cmp::Reverse;
use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_server::shadow_eval_results::store::{ShadowLegReads, ShadowTargetFilter};
use betterlite_server::shadow_result_aggregation::aggregation::{AttemptAggRow, GroupBy};
use betterlite_server::shadow_result_aggregation::error::ShadowResultsError;
use betterlite_server::shadow_result_aggregation::labels::{LabelKind, LabelRow};
use betterlite_server::shadow_result_aggregation::legs::LegRow;
use betterlite_server::shadow_result_aggregation::repository::{
    RepositoryResult, ShadowResultsRepository,
};
use betterlite_server::shadow_result_aggregation::responses::{
    AttemptTotals, ErrorAttempt, LegAttemptCount, latest_error,
};
use betterlite_server::shadow_result_aggregation::results::FunnelTotals;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

use super::payload_optional;
use crate::router_probes::error::OperationError;

// an error attempt seed as the stored row carries it, before the leg id joins it to its job
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorRow {
    job_id: String,
    #[serde(default)]
    error: Option<String>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct LabelRows {
    keys: Vec<LabelRow>,
    teams: Vec<LabelRow>,
    users: Vec<LabelRow>,
}

// the seeded rows every read answers with, so the probe exercises the services, not a query engine
pub(super) struct ProbeStore {
    pub(super) legs: Vec<LegRow>,
    counts: Vec<LegAttemptCount>,
    totals: Option<AttemptTotals>,
    errors: Vec<(String, ErrorAttempt)>,
    by_tier: Vec<AttemptAggRow>,
    by_model: Vec<AttemptAggRow>,
    by_leg: Vec<AttemptAggRow>,
    by_router: Vec<AttemptAggRow>,
    funnel: Option<FunnelTotals>,
    keys: Vec<LabelRow>,
    teams: Vec<LabelRow>,
    users: Vec<LabelRow>,
}

impl ProbeStore {
    pub(super) fn build(payload: &Value) -> Result<Self, OperationError> {
        let rows: LabelRows = payload_optional(payload, "rows")?.unwrap_or_default();
        let error_rows: Vec<ErrorRow> = payload_optional(payload, "errors")?.unwrap_or_default();
        let errors: Vec<(String, ErrorAttempt)> = error_rows
            .into_iter()
            .map(|row| {
                let attempt = ErrorAttempt {
                    error: row.error,
                    created_at: row.created_at,
                };
                (row.job_id, attempt)
            })
            .collect();
        Ok(Self {
            legs: payload_optional(payload, "jobs")?.unwrap_or_default(),
            counts: payload_optional(payload, "counts")?.unwrap_or_default(),
            totals: payload_optional(payload, "totals")?,
            errors,
            by_tier: payload_optional(payload, "by_tier")?.unwrap_or_default(),
            by_model: payload_optional(payload, "by_model")?.unwrap_or_default(),
            by_leg: payload_optional(payload, "by_leg")?.unwrap_or_default(),
            by_router: payload_optional(payload, "by_router")?.unwrap_or_default(),
            funnel: payload_optional(payload, "funnel")?,
            keys: rows.keys,
            teams: rows.teams,
            users: rows.users,
        })
    }
}

#[async_trait]
impl ShadowLegReads for ProbeStore {
    async fn list_legs(
        &self,
        limit: i64,
        target: Option<ShadowTargetFilter>,
    ) -> RepositoryResult<Vec<LegRow>> {
        let mut selected: BTreeMap<String, Vec<&LegRow>> = BTreeMap::new();
        for leg in &self.legs {
            let matches = target.as_ref().is_none_or(|filter| {
                leg.target_type == filter.target_type && leg.target_id == filter.target_id
            });
            if matches {
                selected.entry(leg.group_id.clone()).or_default().push(leg);
            }
        }
        let mut groups: Vec<(String, Vec<&LegRow>)> = selected.into_iter().collect();
        groups.sort_by_key(|(_group, legs)| Reverse(legs.iter().map(|leg| leg.created_at).max()));
        let keep = usize::try_from(limit).unwrap_or(usize::MAX);
        let wanted: Vec<&str> = groups
            .iter()
            .take(keep)
            .map(|(group, _legs)| group.as_str())
            .collect();
        Ok(self
            .legs
            .iter()
            .filter(|leg| wanted.contains(&leg.group_id.as_str()))
            .cloned()
            .collect())
    }

    async fn legs_of_group(&self, job_id: &str) -> RepositoryResult<Vec<LegRow>> {
        let mut legs = Vec::new();
        for leg in &self.legs {
            if leg.group_id == job_id {
                legs.push(leg.clone());
            }
        }
        Ok(legs)
    }
}

#[async_trait]
impl ShadowResultsRepository for ProbeStore {
    async fn attempt_aggregates(
        &self,
        _leg_ids: &[String],
        group_by: GroupBy,
    ) -> RepositoryResult<Vec<AttemptAggRow>> {
        let rows = match group_by {
            GroupBy::Tier => &self.by_tier,
            GroupBy::Model => &self.by_model,
            GroupBy::Leg => &self.by_leg,
            GroupBy::Router => &self.by_router,
        };
        Ok(rows.clone())
    }

    async fn attempt_totals(&self, _leg_ids: &[String]) -> RepositoryResult<AttemptTotals> {
        self.totals.ok_or(ShadowResultsError::NoTotalsRow)
    }

    async fn attempt_count_rows(
        &self,
        leg_ids: &[String],
    ) -> RepositoryResult<Vec<LegAttemptCount>> {
        let mut found = Vec::new();
        for row in &self.counts {
            if leg_ids.contains(&row.job_id) {
                found.push(row.clone());
            }
        }
        Ok(found)
    }

    async fn funnel_totals(&self, _leg_ids: &[String]) -> RepositoryResult<Option<FunnelTotals>> {
        Ok(self.funnel)
    }

    async fn latest_error(&self, leg_ids: &[String]) -> RepositoryResult<Option<String>> {
        let attempts: Vec<ErrorAttempt> = self
            .errors
            .iter()
            .filter(|(job_id, _attempt)| leg_ids.contains(job_id))
            .map(|(_job_id, attempt)| attempt.clone())
            .collect();
        Ok(latest_error(&attempts))
    }

    async fn label_rows(
        &self,
        kind: LabelKind,
        target_ids: &[String],
    ) -> RepositoryResult<Vec<LabelRow>> {
        let rows = match kind {
            LabelKind::Key => &self.keys,
            LabelKind::Team => &self.teams,
            LabelKind::User => &self.users,
        };
        Ok(rows
            .iter()
            .filter(|row| target_ids.contains(&row.target_id))
            .cloned()
            .collect())
    }
}
