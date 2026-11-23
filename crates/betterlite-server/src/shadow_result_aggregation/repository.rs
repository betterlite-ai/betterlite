use async_trait::async_trait;

use super::aggregation::{AttemptAggRow, GroupBy};
use super::error::ShadowResultsError;
use super::labels::{LabelKind, LabelRow, label_responses, target_ids_of};
use super::legs::LegRow;
use super::responses::{AttemptTotals, LegAttemptCount, LegCounts};
use super::results::{FunnelTotals, JobAggregates, JobResults, stratified_results};
use crate::shadow_eval_models::ShadowEvalTargetType;
use crate::shadow_eval_models::job::ShadowJob;

pub type RepositoryResult<Value> = Result<Value, ShadowResultsError>;

#[async_trait]
pub trait ShadowResultsRepository: Send + Sync {
    async fn attempt_aggregates(
        &self,
        leg_ids: &[String],
        group_by: GroupBy,
    ) -> RepositoryResult<Vec<AttemptAggRow>>;

    async fn attempt_totals(&self, leg_ids: &[String]) -> RepositoryResult<AttemptTotals>;

    async fn attempt_count_rows(
        &self,
        leg_ids: &[String],
    ) -> RepositoryResult<Vec<LegAttemptCount>>;

    async fn funnel_totals(&self, leg_ids: &[String]) -> RepositoryResult<Option<FunnelTotals>>;

    async fn latest_error(&self, leg_ids: &[String]) -> RepositoryResult<Option<String>>;

    async fn label_rows(
        &self,
        kind: LabelKind,
        target_ids: &[String],
    ) -> RepositoryResult<Vec<LabelRow>>;

    // _leg_attempt_counts: every leg's figures in one grouped read, failed attempts included
    async fn leg_attempt_counts(&self, legs: &[LegRow]) -> RepositoryResult<LegCounts> {
        let ids = legs.iter().map(|leg| leg.id.clone()).collect::<Vec<_>>();
        if ids.is_empty() {
            return Ok(LegCounts::new());
        }
        let rows = self.attempt_count_rows(&ids).await?;
        Ok(rows
            .into_iter()
            .map(|row| (row.job_id.clone(), row))
            .collect())
    }

    // _with_target_labels: one read per kind, skipped when the responses scope no target of it
    async fn labeled_responses(&self, responses: &[ShadowJob]) -> RepositoryResult<Vec<ShadowJob>> {
        if responses.is_empty() {
            return Ok(Vec::new());
        }
        let key_ids = target_ids_of(responses, ShadowEvalTargetType::Key);
        let team_ids = target_ids_of(responses, ShadowEvalTargetType::Team);
        let user_ids = target_ids_of(responses, ShadowEvalTargetType::User);
        let key_rows = if key_ids.is_empty() {
            Vec::new()
        } else {
            self.label_rows(LabelKind::Key, &key_ids).await?
        };
        let team_rows = if team_ids.is_empty() {
            Vec::new()
        } else {
            self.label_rows(LabelKind::Team, &team_ids).await?
        };
        let user_rows = if user_ids.is_empty() {
            Vec::new()
        } else {
            self.label_rows(LabelKind::User, &user_ids).await?
        };
        Ok(label_responses(
            responses, &key_rows, &team_rows, &user_rows,
        ))
    }

    // _shadow_eval_results: the tier read decides whether there is a result at all
    async fn job_results(&self, legs: &[LegRow]) -> RepositoryResult<JobResults> {
        let leg_ids = legs.iter().map(|leg| leg.id.clone()).collect::<Vec<_>>();
        let by_tier = self.attempt_aggregates(&leg_ids, GroupBy::Tier).await?;
        if by_tier.is_empty() {
            return Ok(JobResults::default());
        }
        let by_model = self.attempt_aggregates(&leg_ids, GroupBy::Model).await?;
        let by_leg = self.attempt_aggregates(&leg_ids, GroupBy::Leg).await?;
        let by_router = self.attempt_aggregates(&leg_ids, GroupBy::Router).await?;
        let funnel = self.funnel_totals(&leg_ids).await?;
        let aggregates = JobAggregates {
            by_tier: &by_tier,
            by_model: &by_model,
            by_leg: &by_leg,
            by_router: &by_router,
            funnel,
        };
        stratified_results(legs, &aggregates)
    }
}
