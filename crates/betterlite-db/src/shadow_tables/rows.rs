use std::error::Error as StdError;
use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

pub type ShadowFuture<'database, Value, Failure> =
    Pin<Box<dyn Future<Output = Result<Value, Failure>> + Send + 'database>>;

// one LiteLLM_ShadowEvalJob row; the direction stays raw text until the sampling side validates it
#[derive(Clone, Debug)]
pub struct ShadowJobRow {
    pub id: String,
    pub target_type: String,
    pub target_id: String,
    pub router_name: String,
    pub router_names: Vec<String>,
    pub models: Vec<String>,
    pub direction: String,
    pub baseline_model: Option<String>,
    pub shadow_percentage: f64,
    pub judge_model: String,
    pub max_turns: i64,
    pub max_budget: Option<Decimal>,
    pub ends_at: DateTime<Utc>,
}

// the group_by row: one attempt count and the summed eval-leg costs for one job
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobAttemptStats {
    pub job_id: String,
    pub attempts: i64,
    pub spend: Decimal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptOutcome {
    Real,
    Shadow,
    Tie,
    Error,
}

impl AttemptOutcome {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Real => "real",
            Self::Shadow => "shadow",
            Self::Tie => "tie",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ShadowAttemptRow {
    pub job_id: String,
    pub request_id: String,
    pub router_name: String,
    pub outcome: AttemptOutcome,
    pub tier: Option<String>,
    pub real_model: Option<String>,
    pub shadow_model: Option<String>,
    pub confidence: Option<Decimal>,
    pub judge_cost: Decimal,
    pub shadow_cost: Decimal,
    pub shadow_classifier_cost: Decimal,
    pub real_cost: Option<Decimal>,
    pub real_classifier_cost: Decimal,
    pub real_cache_hit: bool,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FunnelStage {
    NotSampled,
    Unjudgeable,
    Shed,
    Withheld,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FunnelCounters {
    pub not_sampled: i32,
    pub unjudgeable: i32,
    pub shed: i32,
    pub withheld: i32,
}

impl FunnelCounters {
    pub const fn record(&mut self, stage: FunnelStage) {
        let slot = match stage {
            FunnelStage::NotSampled => &mut self.not_sampled,
            FunnelStage::Unjudgeable => &mut self.unjudgeable,
            FunnelStage::Shed => &mut self.shed,
            FunnelStage::Withheld => &mut self.withheld,
        };
        *slot = slot.saturating_add(1);
    }

    #[must_use]
    pub fn total(self) -> i64 {
        [self.not_sampled, self.unjudgeable, self.shed, self.withheld]
            .into_iter()
            .map(i64::from)
            .fold(0_i64, i64::saturating_add)
    }
}

pub trait ShadowTables: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    fn find_active_jobs(
        &self,
        now: DateTime<Utc>,
    ) -> ShadowFuture<'_, Vec<ShadowJobRow>, Self::Error>;

    fn attempt_stats<'database>(
        &'database self,
        job_ids: &'database [String],
    ) -> ShadowFuture<'database, Vec<JobAttemptStats>, Self::Error>;

    fn insert_attempt<'database>(
        &'database self,
        attempt: &'database ShadowAttemptRow,
    ) -> ShadowFuture<'database, (), Self::Error>;

    fn upsert_funnel<'database>(
        &'database self,
        job_id: &'database str,
        counters: FunnelCounters,
    ) -> ShadowFuture<'database, (), Self::Error>;
}
