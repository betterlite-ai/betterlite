use betterlite_anthropic::anthropic_capability_catalog::{CatalogSources, EffortThresholds};
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use serde_json::Value;

use super::Step;
use super::fixture::NoHfTransport;
use super::thinking::ThinkingFunction;
use super::warnings::Warnings;
use crate::helpers::HelperError;

pub(super) type Environment<'env> = CapabilityEnvironment<'env, NoDynamicMetadata, NoHfTransport>;

pub(super) struct Adapter<'helper> {
    pub(super) catalogs: CatalogSources<'helper>,
    pub(super) thresholds: EffortThresholds,
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
            Step::ProviderQualifiedCatalog(catalog) => Ok(catalog.answer(self, environment).await),
            Step::AdaptiveThinking(thinking) => {
                thinking
                    .answer(ThinkingFunction::Translate, self, environment)
                    .await
            }
            Step::ForcedToolDowngrade(forced) => Ok(forced.answer(self)),
            Step::SamplingCapability(sampling) => sampling.answer(self),
            Step::LegacyBudgetEffort(budget) => Ok(budget.answer(self, environment).await),
            Step::DisabledThinkingWarning(thinking) => {
                thinking
                    .answer(ThinkingFunction::DropDisabled, self, environment)
                    .await
            }
            Step::EffortBetaGate(thinking) => {
                thinking
                    .answer(ThinkingFunction::EffortUsed, self, environment)
                    .await
            }
        }
    }
}
