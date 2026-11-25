use async_trait::async_trait;
use betterlite_core::shadow_evidence::records::{CallFailure, JudgeVerdict, ShadowResponse};
use betterlite_db::shadow_tables::rows::{AttemptOutcome, ShadowAttemptRow, ShadowTables};
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use crate::shadow_calls::judge_call::call_judge;
use crate::shadow_calls::router_call::call_router_shadow;
use crate::shadow_calls::{JudgeDeps, JudgeRequest, ShadowDeps, ShadowRequest};
use crate::shadow_eval_job_loading::attempt::{SpendCounter, attempt_tier, record_attempt};
use crate::shadow_eval_job_loading::jobs::ActiveShadowEvalJob;
use crate::shadow_evaluation::ShadowSample;

#[cfg(test)]
mod tests;

// a stage either returns its outcome or raises before one exists; the two record different rows, so the arm keeps them apart
pub enum ArmFailure {
    Raised(String),
    Failed(CallFailure),
}

pub type ArmStage<Value> = Result<Value, ArmFailure>;

// _call_router_shadow and _call_judge as the arm sees them, so a raising stage and an in-band _CallFailure stay distinguishable
#[async_trait]
pub trait ArmCalls: Send + Sync {
    async fn shadow(&self, request: &ShadowRequest<'_>) -> ArmStage<ShadowResponse>;
    async fn judge(&self, request: &JudgeRequest<'_>) -> ArmStage<JudgeVerdict>;
}

// the production calls: B4084 turns every provider, dispatch and verdict failure into a _CallFailure with its own wording and cost
pub struct RouterCalls<'deps> {
    pub shadow: ShadowDeps<'deps>,
    pub judge: JudgeDeps<'deps>,
}

#[async_trait]
impl ArmCalls for RouterCalls<'_> {
    async fn shadow(&self, request: &ShadowRequest<'_>) -> ArmStage<ShadowResponse> {
        call_router_shadow(request, &self.shadow)
            .await
            .map_err(ArmFailure::Failed)
    }

    async fn judge(&self, request: &JudgeRequest<'_>) -> ArmStage<JudgeVerdict> {
        call_judge(request, &self.judge)
            .await
            .map_err(ArmFailure::Failed)
    }
}

pub struct AttemptDeps<'deps, Database, Counter> {
    pub database: Option<&'deps Database>,
    pub counter: &'deps Counter,
    pub calls: &'deps dyn ArmCalls,
}

// _run_shadow_arm: shadow call, blind judge, then one attempt row at every exit, so one arm's fault never silences a sibling
pub async fn run_shadow_arm<Database, Counter>(
    job: &ActiveShadowEvalJob,
    arm_router: &str,
    sample: &ShadowSample<'_>,
    deps: &AttemptDeps<'_, Database, Counter>,
) where
    Database: ShadowTables,
    Counter: SpendCounter,
{
    let shadow_request = ShadowRequest {
        target_model: arm_target(job, arm_router),
        messages: sample.messages,
        shadow_params: sample.shadow_params,
        parent_metadata: sample.parent_metadata,
    };
    let shadow = match deps.calls.shadow(&shadow_request).await {
        Ok(shadow) => shadow,
        Err(failure) => {
            record_shadow_failure(job, arm_router, sample, deps, failure).await;
            return;
        }
    };
    let no_tools = Json::Null;
    let judge_request = JudgeRequest {
        judge_model: &job.judge_model,
        messages: sample.messages,
        real_text: sample.real_text,
        shadow_text: &shadow.text,
        tools: sample.shadow_params.get("tools").unwrap_or(&no_tools),
        parent_metadata: sample.parent_metadata,
    };
    let verdict = deps.calls.judge(&judge_request).await;
    record_judge_outcome(job, arm_router, sample, deps, &shadow, verdict).await;
}

// ActiveShadowEvalJob.arm_target: a reverse job calls its fixed baseline, a forward job the candidate router itself
fn arm_target<'job>(job: &'job ActiveShadowEvalJob, arm_router: &'job str) -> &'job str {
    job.baseline_model.as_deref().unwrap_or(arm_router)
}

async fn record_shadow_failure<Database, Counter>(
    job: &ActiveShadowEvalJob,
    arm_router: &str,
    sample: &ShadowSample<'_>,
    deps: &AttemptDeps<'_, Database, Counter>,
    failure: ArmFailure,
) where
    Database: ShadowTables,
    Counter: SpendCounter,
{
    let row = match failure {
        ArmFailure::Raised(message) => {
            tracing::debug!(
                "shadow_eval: pipeline failed for {}: {message}",
                sample.request_id
            );
            pipeline_row(job, arm_router, sample, &message, None)
        }
        ArmFailure::Failed(failure) => {
            let mut row = attempt_row(job, arm_router, sample);
            row.shadow_cost = failure.cost;
            row.shadow_classifier_cost = failure.classifier_cost;
            row.error = Some(failure.error);
            row
        }
    };
    record_attempt(deps.database, deps.counter, row).await;
}

async fn record_judge_outcome<Database, Counter>(
    job: &ActiveShadowEvalJob,
    arm_router: &str,
    sample: &ShadowSample<'_>,
    deps: &AttemptDeps<'_, Database, Counter>,
    shadow: &ShadowResponse,
    verdict: ArmStage<JudgeVerdict>,
) where
    Database: ShadowTables,
    Counter: SpendCounter,
{
    let row = match verdict {
        Ok(verdict) => success_row(job, arm_router, sample, shadow, &verdict),
        Err(ArmFailure::Failed(failure)) => {
            let mut row = billed_row(job, arm_router, sample, shadow);
            row.judge_cost = failure.cost;
            row.error = Some(failure.error);
            row
        }
        Err(ArmFailure::Raised(message)) => {
            tracing::debug!(
                "shadow_eval: pipeline failed for {}: {message}",
                sample.request_id
            );
            pipeline_row(job, arm_router, sample, &message, Some(shadow))
        }
    };
    record_attempt(deps.database, deps.counter, row).await;
}

fn attempt_row(
    job: &ActiveShadowEvalJob,
    arm_router: &str,
    sample: &ShadowSample<'_>,
) -> ShadowAttemptRow {
    ShadowAttemptRow {
        job_id: job.id.clone(),
        request_id: sample.request_id.to_owned(),
        router_name: arm_router.to_owned(),
        outcome: AttemptOutcome::Error,
        tier: attempt_tier(job.direction, sample.control_tier.map(str::to_owned), None),
        real_model: None,
        shadow_model: None,
        confidence: None,
        judge_cost: Decimal::ZERO,
        shadow_cost: Decimal::ZERO,
        shadow_classifier_cost: Decimal::ZERO,
        real_cost: Some(sample.real_cost),
        real_classifier_cost: sample.real_classifier_cost,
        real_cache_hit: sample.real_cache_hit,
        error: None,
    }
}

// a row whose shadow call billed: the tier and served model come from the shadow response
fn billed_row(
    job: &ActiveShadowEvalJob,
    arm_router: &str,
    sample: &ShadowSample<'_>,
    shadow: &ShadowResponse,
) -> ShadowAttemptRow {
    let mut row = attempt_row(job, arm_router, sample);
    row.tier = attempt_tier(
        job.direction,
        sample.control_tier.map(str::to_owned),
        shadow.tier.clone(),
    );
    row.shadow_model = Some(shadow.model.clone());
    row.shadow_cost = shadow.cost;
    row.shadow_classifier_cost = shadow.classifier_cost;
    row
}

fn success_row(
    job: &ActiveShadowEvalJob,
    arm_router: &str,
    sample: &ShadowSample<'_>,
    shadow: &ShadowResponse,
    verdict: &JudgeVerdict,
) -> ShadowAttemptRow {
    let mut row = billed_row(job, arm_router, sample, shadow);
    row.outcome = preference_outcome(&verdict.preference);
    row.real_model = Some(sample.real_model.to_owned());
    row.confidence = Some(verdict.confidence);
    row.judge_cost = verdict.cost;
    row
}

fn pipeline_row(
    job: &ActiveShadowEvalJob,
    arm_router: &str,
    sample: &ShadowSample<'_>,
    message: &str,
    shadow: Option<&ShadowResponse>,
) -> ShadowAttemptRow {
    let mut row = match shadow {
        Some(shadow) => billed_row(job, arm_router, sample, shadow),
        None => attempt_row(job, arm_router, sample),
    };
    row.error = Some(format!("pipeline error: {message}"));
    row
}

// the judge verdict arrives unmasked as one of the three row outcomes; anything else is LiteLLM's tie
fn preference_outcome(preference: &str) -> AttemptOutcome {
    match preference {
        "shadow" => AttemptOutcome::Shadow,
        "real" => AttemptOutcome::Real,
        _ => AttemptOutcome::Tie,
    }
}
