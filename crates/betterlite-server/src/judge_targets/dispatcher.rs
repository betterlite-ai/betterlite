use betterlite_models::judge_primitives::provider_qualified::ProviderCache;
use betterlite_models::resolution::ResolutionContext;

use crate::judge_targets::router::JudgeRouter;
use crate::judge_targets::sdk::JudgeSdk;

pub struct JudgeDispatcher<'dispatch> {
    pub router: Option<&'dispatch dyn JudgeRouter>,
    pub sdk: &'dispatch dyn JudgeSdk,
    pub cache: &'dispatch ProviderCache,
    pub context: &'dispatch ResolutionContext<'dispatch>,
}
