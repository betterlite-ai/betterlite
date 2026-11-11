use async_trait::async_trait;
use betterlite_models::decisions::Json;

use crate::judge_targets::call::JudgeCall;
use crate::judge_targets::error::JudgeDispatchError;

#[async_trait]
pub trait JudgeSdk: Send + Sync {
    async fn acompletion(&self, call: JudgeCall) -> Result<Json, JudgeDispatchError>;
}
