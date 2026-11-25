use async_trait::async_trait;

use crate::shadow_eval_models::ShadowEvalTargetType;
use crate::shadow_result_aggregation::legs::LegRow;
use crate::shadow_result_aggregation::repository::RepositoryResult;

// the one-target filter _LIST_LEGS_BY_TARGET_SQL applies to whole jobs
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShadowTargetFilter {
    pub target_type: ShadowEvalTargetType,
    pub target_id: String,
}

// the two reads the listing and detail endpoints add to B381's aggregate repository
#[async_trait]
pub trait ShadowLegReads: Send + Sync {
    async fn list_legs(
        &self,
        limit: i64,
        target: Option<ShadowTargetFilter>,
    ) -> RepositoryResult<Vec<LegRow>>;

    async fn legs_of_group(&self, job_id: &str) -> RepositoryResult<Vec<LegRow>>;
}
