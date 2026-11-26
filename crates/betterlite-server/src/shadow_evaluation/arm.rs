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

mod tests;

pub enum ArmFailure {
    Raised(String),
    Failed(CallFailure),
}

pub type ArmStage<Value> = Result<Value, ArmFailure>;

#[async_trait]
pub trait ArmCalls: Send + Sync {
    async fn shadow(&self, request: &ShadowRequest<'_>) -> ArmStage<ShadowResponse>;
    async fn judge(&self, request: &JudgeRequest<'_>) -> ArmStage<JudgeVerdict>;
}

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

struct RowSpec<'row> {
    job: &'row ActiveShadowEvalJob,
    arm_router: &'row str,
    sample: &'row ShadowSample<'row>,
}

impl RowSpec<'_> {
    fn attempt(&self) -> ShadowAttemptRow {
        ShadowAttemptRow {
            job_id: self.job.id.clone(),
            request_id: self.sample.request_id.to_owned(),
            router_name: self.arm_router.to_owned(),
            outcome: AttemptOutcome::Error,
            tier: attempt_tier(
                self.job.direction,
                self.sample.control_tier.map(str::to_owned),
                None,
            ),
            real_model: None,
            shadow_model: None,
            confidence: None,
            judge_cost: Decimal::ZERO,
            shadow_cost: Decimal::ZERO,
            shadow_classifier_cost: Decimal::ZERO,
            real_cost: Some(self.sample.real_cost),
            real_classifier_cost: self.sample.real_classifier_cost,
            real_cache_hit: self.sample.real_cache_hit,
            error: None,
        }
    }

    fn billed(&self, shadow: &ShadowResponse) -> ShadowAttemptRow {
        let mut row = self.attempt();
        row.tier = attempt_tier(
            self.job.direction,
            self.sample.control_tier.map(str::to_owned),
            shadow.tier.clone(),
        );
        row.shadow_model = Some(shadow.model.clone());
        row.shadow_cost = shadow.cost;
        row.shadow_classifier_cost = shadow.classifier_cost;
        row
    }

    fn success(&self, shadow: &ShadowResponse, verdict: &JudgeVerdict) -> ShadowAttemptRow {
        let mut row = self.billed(shadow);
        row.outcome = preference_outcome(&verdict.preference);
        row.real_model = Some(self.sample.real_model.to_owned());
        row.confidence = Some(verdict.confidence);
        row.judge_cost = verdict.cost;
        row
    }

    fn judge_failure(&self, failure: CallFailure, shadow: &ShadowResponse) -> ShadowAttemptRow {
        let mut row = self.billed(shadow);
        row.judge_cost = failure.cost;
        row.error = Some(failure.error);
        row
    }

    fn shadow_failure(&self, failure: ArmFailure) -> ShadowAttemptRow {
        match failure {
            ArmFailure::Raised(message) => self.pipeline(&message, None),
            ArmFailure::Failed(failed) => ShadowAttemptRow {
                shadow_cost: failed.cost,
                shadow_classifier_cost: failed.classifier_cost,
                error: Some(failed.error),
                ..self.attempt()
            },
        }
    }

    fn pipeline(&self, message: &str, shadow: Option<&ShadowResponse>) -> ShadowAttemptRow {
        tracing::debug!(
            "shadow_eval: pipeline failed for {}: {message}",
            self.sample.request_id
        );
        let base = shadow.map_or_else(|| self.attempt(), |billed| self.billed(billed));
        ShadowAttemptRow {
            error: Some(format!("pipeline error: {message}")),
            ..base
        }
    }
}

pub async fn run_shadow_arm<Database, Counter>(
    job: &ActiveShadowEvalJob,
    arm_router: &str,
    sample: &ShadowSample<'_>,
    deps: &AttemptDeps<'_, Database, Counter>,
) where
    Database: ShadowTables,
    Counter: SpendCounter,
{
    let rows = RowSpec {
        job,
        arm_router,
        sample,
    };
    let shadow_request = ShadowRequest {
        target_model: job.baseline_model.as_deref().unwrap_or(arm_router),
        messages: sample.messages,
        shadow_params: sample.shadow_params,
        parent_metadata: sample.parent_metadata,
    };
    let row = match deps.calls.shadow(&shadow_request).await {
        Ok(shadow) => judge_row(&rows, &shadow, deps.calls).await,
        Err(failure) => rows.shadow_failure(failure),
    };
    record_attempt(deps.database, deps.counter, row).await;
}

async fn judge_row(
    rows: &RowSpec<'_>,
    shadow: &ShadowResponse,
    calls: &dyn ArmCalls,
) -> ShadowAttemptRow {
    let no_tools = Json::Null;
    let judge_request = JudgeRequest {
        judge_model: &rows.job.judge_model,
        messages: rows.sample.messages,
        real_text: rows.sample.real_text,
        shadow_text: &shadow.text,
        tools: rows.sample.shadow_params.get("tools").unwrap_or(&no_tools),
        parent_metadata: rows.sample.parent_metadata,
    };
    match calls.judge(&judge_request).await {
        Ok(verdict) => rows.success(shadow, &verdict),
        Err(ArmFailure::Failed(failure)) => rows.judge_failure(failure, shadow),
        Err(ArmFailure::Raised(message)) => rows.pipeline(&message, Some(shadow)),
    }
}

fn preference_outcome(preference: &str) -> AttemptOutcome {
    match preference {
        "shadow" => AttemptOutcome::Shadow,
        "real" => AttemptOutcome::Real,
        _ => AttemptOutcome::Tie,
    }
}
