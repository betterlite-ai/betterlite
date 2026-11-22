use async_trait::async_trait;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::shadow_result_aggregation::aggregation::{AttemptAggRow, GroupBy};
use betterlite_server::shadow_result_aggregation::error::ShadowResultsError;
use betterlite_server::shadow_result_aggregation::labels::{LabelKind, LabelRow};
use betterlite_server::shadow_result_aggregation::legs::LegRow;
use betterlite_server::shadow_result_aggregation::repository::{
    RepositoryResult, ShadowResultsRepository,
};
use betterlite_server::shadow_result_aggregation::responses::{
    AttemptTotals, ErrorAttempt, LegAttemptCount, detail_response, group_response, latest_error,
};
use betterlite_server::shadow_result_aggregation::results::FunnelTotals;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

// the rows the detail read hands each LiteLLM query, so the fake prisma client answers with them
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    group_id: String,
    legs: Vec<LegRow>,
    counts: Vec<LegAttemptCount>,
    totals: AttemptTotals,
    errors: Vec<AttemptError>,
    by_tier: Vec<AttemptAggRow>,
    by_model: Vec<AttemptAggRow>,
    by_leg: Vec<AttemptAggRow>,
    by_router: Vec<AttemptAggRow>,
    funnel: FunnelTotals,
    keys: Vec<KeyRow>,
    teams: Vec<TeamRow>,
    users: Vec<UserRow>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AttemptError {
    job_id: String,
    error: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyRow {
    token: String,
    key_alias: Option<String>,
    key_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TeamRow {
    team_id: String,
    team_alias: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UserRow {
    user_id: String,
    user_email: Option<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_config_error()?;
        runtime.block_on(answer(input)).or_value_error()
    }
}

// get_shadow_eval_job's composition: counts, labels, totals, latest error, then stratified results
async fn answer(input: &Input) -> Result<Value, ShadowResultsError> {
    let repo = FakeRepo { input };
    let legs = &input.legs;
    let leg_ids: Vec<String> = legs.iter().map(|leg| leg.id.clone()).collect();
    let counts = repo.leg_attempt_counts(legs).await?;
    let grouped = group_response(&input.group_id, legs, &counts)?;
    let labeled = repo.labeled_responses(&[grouped]).await?;
    let totals = repo.attempt_totals(&leg_ids).await?;
    let last_error = repo.latest_error(&leg_ids).await?;
    let results = repo.job_results(legs).await?;
    let job = labeled
        .into_iter()
        .next()
        .ok_or(ShadowResultsError::NoLegs)?;
    Ok(json!({
        "response": detail_response(&job, totals, last_error, &results),
    }))
}

struct FakeRepo<'input> {
    input: &'input Input,
}

#[async_trait]
impl ShadowResultsRepository for FakeRepo<'_> {
    async fn attempt_aggregates(
        &self,
        _leg_ids: &[String],
        group_by: GroupBy,
    ) -> RepositoryResult<Vec<AttemptAggRow>> {
        let rows = match group_by {
            GroupBy::Tier => &self.input.by_tier,
            GroupBy::Model => &self.input.by_model,
            GroupBy::Leg => &self.input.by_leg,
            GroupBy::Router => &self.input.by_router,
        };
        Ok(rows.clone())
    }

    async fn attempt_totals(&self, _leg_ids: &[String]) -> RepositoryResult<AttemptTotals> {
        Ok(self.input.totals)
    }

    async fn attempt_count_rows(
        &self,
        leg_ids: &[String],
    ) -> RepositoryResult<Vec<LegAttemptCount>> {
        let mut counts = Vec::new();
        for row in &self.input.counts {
            if leg_ids.contains(&row.job_id) {
                counts.push(row.clone());
            }
        }
        Ok(counts)
    }

    async fn funnel_totals(&self, _leg_ids: &[String]) -> RepositoryResult<Option<FunnelTotals>> {
        Ok(Some(self.input.funnel))
    }

    async fn latest_error(&self, leg_ids: &[String]) -> RepositoryResult<Option<String>> {
        let attempts: Vec<ErrorAttempt> = self
            .input
            .errors
            .iter()
            .filter(|row| leg_ids.contains(&row.job_id))
            .map(|row| ErrorAttempt {
                error: row.error.clone(),
                created_at: row.created_at,
            })
            .collect();
        Ok(latest_error(&attempts))
    }

    async fn label_rows(
        &self,
        kind: LabelKind,
        target_ids: &[String],
    ) -> RepositoryResult<Vec<LabelRow>> {
        Ok(self.input.label_rows(kind, target_ids))
    }
}

impl Input {
    fn label_rows(&self, kind: LabelKind, target_ids: &[String]) -> Vec<LabelRow> {
        match kind {
            LabelKind::Key => self
                .keys
                .iter()
                .filter(|row| target_ids.contains(&row.token))
                .map(|row| LabelRow {
                    target_id: row.token.clone(),
                    target_alias: row.key_alias.clone(),
                    key_name: row.key_name.clone(),
                })
                .collect(),
            LabelKind::Team => self
                .teams
                .iter()
                .filter(|row| target_ids.contains(&row.team_id))
                .map(|row| LabelRow {
                    target_id: row.team_id.clone(),
                    target_alias: row.team_alias.clone(),
                    key_name: None,
                })
                .collect(),
            LabelKind::User => self
                .users
                .iter()
                .filter(|row| target_ids.contains(&row.user_id))
                .map(|row| LabelRow {
                    target_id: row.user_id.clone(),
                    target_alias: row.user_email.clone(),
                    key_name: None,
                })
                .collect(),
        }
    }
}
