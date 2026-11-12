use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::capabilities::ThinkingBudgets;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use serde_json::Value;

use super::Step;
use super::fixture::NoHfTransport;
use super::warnings::Warnings;
use crate::helpers::HelperError;

pub(super) type Environment<'env> = CapabilityEnvironment<'env, NoDynamicMetadata, NoHfTransport>;

pub(super) struct Adapter<'helper> {
    pub(super) catalogs: CatalogSources<'helper>,
    pub(super) budgets: ThinkingBudgets,
    pub(super) warnings: &'helper Warnings,
}

impl Adapter<'_> {
    pub(super) async fn answer(
        &self,
        step: &Step,
        environment: &mut Environment<'_>,
    ) -> Result<Value, Box<HelperError>> {
        self.warnings.clear();
        match step {
            Step::CapThinkingBudgetToMaxTokens(call) => call.answer(),
            Step::GetSupportedOpenaiParams(call) => call.answer(self, environment).await,
            Step::MapReasoningEffort(call) => call.answer(self, environment).await,
            Step::MaybeDropSpeedParam(call) => call.answer(self),
            Step::ModelSupportsEffortParam(call) => Ok(call.answer(self, environment).await),
            Step::ModelSupportsSpeedParam(call) => Ok(call.answer(self)),
            Step::RaiseInvalidReasoningEffort(call) => Ok(call.answer()),
            Step::ValidateEffortForModel(call) => Ok(call.answer(self, environment).await),
        }
    }
}
