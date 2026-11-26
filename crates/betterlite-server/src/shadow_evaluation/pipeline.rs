use std::future::Future;
use std::pin::Pin;

use betterlite_auth::auth_key_budgets::spend::SpendReadError;
use betterlite_db::shadow_tables::funnel::FunnelQueue;
use betterlite_db::shadow_tables::rows::{FunnelStage, ShadowTables};
use rust_decimal::Decimal;

use crate::shadow_eval_job_loading::admission::job_spend_counter_key;
use crate::shadow_eval_job_loading::attempt::SpendCounter;
use crate::shadow_eval_job_loading::jobs::ActiveShadowEvalJob;
use crate::shadow_evaluation::ShadowSample;
use crate::shadow_evaluation::advisory_budget::{AdvisoryDeps, key_or_team_over_budget};
use crate::shadow_evaluation::arm::{ArmCalls, AttemptDeps, run_shadow_arm};

mod tests;

pub trait FunnelRecorder: Send + Sync {
    fn record(&self, job_id: &str, stage: FunnelStage);
}

impl FunnelRecorder for FunnelQueue {
    fn record(&self, job_id: &str, stage: FunnelStage) {
        Self::record(self, job_id, stage);
    }
}

pub type JobSpendFuture<'reader> =
    Pin<Box<dyn Future<Output = Result<Decimal, SpendReadError>> + Send + 'reader>>;

pub trait JobSpendReader: Send + Sync {
    fn read_spend<'reader>(
        &'reader self,
        counter_key: &'reader str,
        fallback_spend: Decimal,
        max_budget: Decimal,
    ) -> JobSpendFuture<'reader>;
}

pub struct PipelineDeps<'deps, Database, Counter> {
    pub database: Option<&'deps Database>,
    pub counter: &'deps Counter,
    pub funnel: &'deps dyn FunnelRecorder,
    pub advisory: &'deps AdvisoryDeps<'deps>,
    pub spend: &'deps dyn JobSpendReader,
    pub calls: &'deps dyn ArmCalls,
}

pub async fn run_shadow_eval<Database, Counter>(
    job: &ActiveShadowEvalJob,
    sample: &ShadowSample<'_>,
    deps: &PipelineDeps<'_, Database, Counter>,
) where
    Database: ShadowTables,
    Counter: SpendCounter,
{
    let sampled = deps.database.is_some()
        && !key_or_team_over_budget(sample.parent_metadata, deps.advisory).await
        && budget_allows(job, deps.spend).await;
    if !sampled {
        deps.funnel.record(&job.id, FunnelStage::Withheld);
        return;
    }
    let attempts = AttemptDeps {
        database: deps.database,
        counter: deps.counter,
        calls: deps.calls,
    };
    for arm_router in &job.arm_router_names {
        run_shadow_arm(job, arm_router, sample, &attempts).await;
    }
}

async fn budget_allows(job: &ActiveShadowEvalJob, spend: &dyn JobSpendReader) -> bool {
    let Some(max_budget) = job.max_budget else {
        return true;
    };
    let counter_key = job_spend_counter_key(&job.id);
    match spend.read_spend(&counter_key, job.spend, max_budget).await {
        Ok(current) => current < max_budget,
        Err(error) => {
            tracing::warn!(
                "shadow_eval: budget unverifiable for {}, sample skipped: {error}",
                job.id
            );
            false
        }
    }
}
