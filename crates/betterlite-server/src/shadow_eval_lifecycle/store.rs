use std::error::Error as StdError;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::shadow_eval_job_loading::jobs::ShadowEvalDirection;
use crate::shadow_eval_models::ShadowEvalTargetType;
use crate::shadow_result_aggregation::legs::LegRow;
use crate::shadow_result_aggregation::responses::LegAttemptCount;

// one LiteLLM_VerificationToken row the start endpoint loads for its targets
#[derive(Clone)]
pub struct TargetKeyRow {
    pub token: String,
    pub key_alias: Option<String>,
    pub key_name: Option<String>,
    pub team_id: Option<String>,
}

#[derive(Clone)]
pub struct TargetTeamRow {
    pub team_id: String,
    pub team_alias: Option<String>,
}

#[derive(Clone)]
pub struct TargetUserRow {
    pub user_id: String,
    pub user_email: Option<String>,
}

#[derive(Default)]
pub struct TargetRows {
    pub keys: Vec<TargetKeyRow>,
    pub teams: Vec<TargetTeamRow>,
    pub users: Vec<TargetUserRow>,
}

// one row already holding a target's per-direction slot, as the claim read reports it
#[derive(Clone)]
pub struct ClaimedTarget {
    pub group_id: String,
    pub target_type: ShadowEvalTargetType,
    pub target_id: String,
}

// one leg LiteLLM_ShadowEvalJob row as the start endpoint writes it, every leg sharing one config
pub struct NewLeg {
    pub id: String,
    pub group_id: String,
    pub target_type: ShadowEvalTargetType,
    pub target_id: String,
    pub router_name: String,
    pub router_names: Vec<String>,
    pub models: Vec<String>,
    pub direction: ShadowEvalDirection,
    pub baseline_model: Option<String>,
    pub judge_model: String,
    pub shadow_percentage: f64,
    pub max_turns: i32,
    pub max_budget: Decimal,
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
}

impl NewLeg {
    // the response is built from the values this request wrote, never from a lagging read replica
    #[must_use]
    pub fn row(&self) -> LegRow {
        LegRow {
            id: self.id.clone(),
            group_id: self.group_id.clone(),
            target_type: self.target_type,
            target_id: self.target_id.clone(),
            router_name: self.router_name.clone(),
            router_names: self.router_names.clone(),
            models: self.models.clone(),
            direction: self.direction,
            baseline_model: self.baseline_model.clone(),
            judge_model: self.judge_model.clone(),
            shadow_percentage: self.shadow_percentage,
            max_turns: i64::from(self.max_turns),
            max_budget: Some(self.max_budget),
            created_at: self.created_at,
            ends_at: self.ends_at,
            stopped_at: None,
            stopped_by: None,
        }
    }
}

// a leg insert either landed or lost the partial unique index to a concurrent start
pub enum InsertRejection<DatabaseError> {
    UniqueViolation,
    Database(DatabaseError),
}

#[async_trait]
pub trait ShadowLifecycleStore: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    async fn load_targets(
        &self,
        keys: &[String],
        teams: &[String],
        users: &[String],
    ) -> Result<TargetRows, Self::Error>;

    async fn sweep_finished(
        &self,
        ids: &[String],
        target_type: ShadowEvalTargetType,
    ) -> Result<(), Self::Error>;

    async fn claimed_targets(
        &self,
        targets: &[(ShadowEvalTargetType, Vec<String>)],
        direction: ShadowEvalDirection,
    ) -> Result<Vec<ClaimedTarget>, Self::Error>;

    async fn create_legs(&self, legs: &[NewLeg]) -> Result<(), InsertRejection<Self::Error>>;

    async fn seed_funnel(&self, job_ids: &[String]) -> Result<(), Self::Error>;

    async fn legs(&self, group_id: &str) -> Result<Vec<LegRow>, Self::Error>;

    async fn attempt_stats(&self, job_ids: &[String]) -> Result<Vec<LegAttemptCount>, Self::Error>;

    async fn stop_job(
        &self,
        job_id: &str,
        operator: &str,
        stamp: DateTime<Utc>,
    ) -> Result<u64, Self::Error>;
}
