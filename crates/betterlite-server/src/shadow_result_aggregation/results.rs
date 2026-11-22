use std::collections::BTreeMap;

use num_traits::ToPrimitive as _;
use rust_decimal::Decimal;
use serde::Deserialize;

use super::aggregation::{AttemptAggRow, pct_of, slices};
use super::error::ShadowResultsError;
use super::legs::LegRow;
use super::targets::TargetKey;
use crate::shadow_eval_models::results::{ShadowEvalResult, ShadowEvalSlice};

// the funnel table's counts for a job, with the row count that alone proves every leg reported
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FunnelTotals {
    pub legs_with_rows: i64,
    pub not_sampled: i64,
    pub unjudgeable: i64,
    pub shed: i64,
    pub withheld: i64,
}

pub type TargetVerdicts = BTreeMap<TargetKey, ShadowEvalSlice>;

pub struct JobAggregates<'rows> {
    pub by_tier: &'rows [AttemptAggRow],
    pub by_model: &'rows [AttemptAggRow],
    pub by_leg: &'rows [AttemptAggRow],
    pub by_router: &'rows [AttemptAggRow],
    pub funnel: Option<FunnelTotals>,
}

#[derive(Debug, Default)]
pub struct JobResults {
    pub results: Option<ShadowEvalResult>,
    pub verdicts: TargetVerdicts,
}

// _shadow_eval_results: no judged turn means no result, and only then the other reads are skipped
pub fn stratified_results(
    legs: &[LegRow],
    aggregates: &JobAggregates<'_>,
) -> Result<JobResults, ShadowResultsError> {
    if aggregates.by_tier.is_empty() {
        return Ok(JobResults::default());
    }
    let verdicts = target_verdicts(legs, aggregates.by_leg)?;
    Ok(JobResults {
        results: Some(job_result(legs, aggregates)?),
        verdicts,
    })
}

fn job_result(
    legs: &[LegRow],
    aggregates: &JobAggregates<'_>,
) -> Result<ShadowEvalResult, ShadowResultsError> {
    let totals = OverallTotals::of(aggregates.by_tier)?;
    // coverage only when every leg has a funnel row; a partial seed reads as unknown, not as a sum
    let funnel = aggregates.funnel.filter(|counts| {
        usize::try_from(counts.legs_with_rows).is_ok_and(|rows| rows == legs.len())
    });
    Ok(ShadowEvalResult {
        by_tier: slices(aggregates.by_tier)?,
        by_current_model: slices(aggregates.by_model)?,
        by_router: slices(aggregates.by_router)?,
        overall_shadow_win_rate_pct: pct_of(totals.shadow_wins, totals.turns)?,
        overall_tie_rate_pct: pct_of(totals.ties, totals.turns)?,
        sampled_real_spend: totals.real_spend,
        sampled_shadow_spend: totals.shadow_spend,
        not_sampled_count: funnel.map(|counts| counts.not_sampled),
        unjudgeable_count: funnel.map(|counts| counts.unjudgeable),
        shed_count: funnel.map(|counts| counts.shed),
        withheld_count: funnel.map(|counts| counts.withheld),
    })
}

// the job-wide denominators and spends, summed over the tier rows in the order the read gave them
struct OverallTotals {
    turns: i64,
    shadow_wins: i64,
    ties: i64,
    real_spend: Decimal,
    shadow_spend: Decimal,
}

impl OverallTotals {
    fn of(rows: &[AttemptAggRow]) -> Result<Self, ShadowResultsError> {
        let turns = rows
            .iter()
            .map(|row| row.turn_count)
            .fold(0_i64, i64::saturating_add);
        let shadow_wins = rows
            .iter()
            .map(|row| row.shadow_wins)
            .fold(0_i64, i64::saturating_add);
        let ties = rows
            .iter()
            .map(|row| row.ties)
            .fold(0_i64, i64::saturating_add);
        Ok(Self {
            turns,
            shadow_wins,
            ties,
            real_spend: summed_spend(rows, |row| row.real_spend, "sampled_real_spend")?,
            shadow_spend: summed_spend(rows, |row| row.shadow_spend, "sampled_shadow_spend")?,
        })
    }
}

// LiteLLM adds these doubles in order; the response serializes that sum, so f64 does the adding
fn summed_spend(
    rows: &[AttemptAggRow],
    of: fn(&AttemptAggRow) -> Decimal,
    name: &'static str,
) -> Result<Decimal, ShadowResultsError> {
    let total = rows
        .iter()
        .map(|row| of(row).to_f64().unwrap_or(f64::NAN))
        .sum::<f64>();
    let exact = Decimal::from_f64_retain(total);
    exact.ok_or(ShadowResultsError::Unrepresentable { name })
}

// each leg's slice, keyed by the (target_type, target_id) pair so same-id targets never collapse
fn target_verdicts(
    legs: &[LegRow],
    by_leg: &[AttemptAggRow],
) -> Result<TargetVerdicts, ShadowResultsError> {
    let targets: BTreeMap<&str, &LegRow> = legs.iter().map(|leg| (leg.id.as_str(), leg)).collect();
    let mut verdicts = TargetVerdicts::new();
    for row in slices(by_leg)? {
        let leg =
            targets
                .get(row.group.as_str())
                .ok_or_else(|| ShadowResultsError::UnknownLeg {
                    leg_id: row.group.clone(),
                })?;
        let key = TargetKey {
            target_type: leg.target_type,
            target_id: leg.target_id.clone(),
        };
        verdicts.insert(
            key,
            ShadowEvalSlice {
                group: leg.target_id.clone(),
                ..row
            },
        );
    }
    Ok(verdicts)
}
