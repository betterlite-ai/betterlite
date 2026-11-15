use std::collections::BTreeSet;

use betterlite_db::shadow_tables::rows::ShadowJobRow;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowEvalDirection {
    Forward,
    Reverse,
}

impl ShadowEvalDirection {
    pub fn parse(raw: &str) -> Result<Self, ShadowJobError> {
        match raw {
            "forward" => Ok(Self::Forward),
            "reverse" => Ok(Self::Reverse),
            other => Err(ShadowJobError::UnknownDirection {
                direction: other.to_owned(),
            }),
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Forward => "forward",
            Self::Reverse => "reverse",
        }
    }
}

#[derive(Debug, Error)]
pub enum ShadowJobError {
    #[error("direction {direction} is not forward or reverse")]
    UnknownDirection { direction: String },
    #[error("baseline_model is set for exactly the reverse jobs")]
    BaselineModelMismatch,
    #[error("a reverse job evaluates exactly one router")]
    ReverseEvaluatesOneRouter,
}

// one active job as the sampling path needs it: config plus the attempt stats at cache fill
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveShadowEvalJob {
    pub id: String,
    pub router_name: String,
    pub arm_router_names: Vec<String>,
    pub models: BTreeSet<String>,
    pub direction: ShadowEvalDirection,
    pub baseline_model: Option<String>,
    pub shadow_percentage: f64,
    pub judge_model: String,
    pub max_turns: i64,
    pub max_budget: Option<Decimal>,
    pub ends_at: DateTime<Utc>,
    pub attempts: i64,
    pub spend: Decimal,
}

impl ActiveShadowEvalJob {
    pub fn from_row(
        row: &ShadowJobRow,
        attempts: i64,
        spend: Decimal,
    ) -> Result<Self, ShadowJobError> {
        let direction = ShadowEvalDirection::parse(&row.direction)?;
        if row.baseline_model.is_some() != (direction == ShadowEvalDirection::Reverse) {
            return Err(ShadowJobError::BaselineModelMismatch);
        }
        let arm_router_names = merge_arm_routers(&row.router_name, &row.router_names);
        if direction == ShadowEvalDirection::Reverse && arm_router_names.len() > 1 {
            return Err(ShadowJobError::ReverseEvaluatesOneRouter);
        }
        Ok(Self {
            id: row.id.clone(),
            router_name: row.router_name.clone(),
            arm_router_names,
            models: row.models.iter().cloned().collect(),
            direction,
            baseline_model: row.baseline_model.clone(),
            shadow_percentage: row.shadow_percentage,
            judge_model: row.judge_model.clone(),
            max_turns: row.max_turns,
            max_budget: row.max_budget,
            ends_at: row.ends_at,
            attempts,
            spend,
        })
    }

    #[must_use]
    pub fn arm_target<'job>(&'job self, arm_router: &'job str) -> &'job str {
        match &self.baseline_model {
            Some(baseline) => baseline.as_str(),
            None => arm_router,
        }
    }
}

// rows from before router_names existed hold the whole router set in router_name alone
fn merge_arm_routers(router_name: &str, router_names: &[String]) -> Vec<String> {
    if router_names.is_empty() {
        return vec![router_name.to_owned()];
    }
    router_names.to_vec()
}
