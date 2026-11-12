use betterlite_anthropic::anthropic_capability_catalog::forced_tool::apply_forced_tool_choice;
use betterlite_anthropic::anthropic_capability_catalog::sampling::{
    SamplingRequest, apply_sampling_param,
};
use betterlite_anthropic::anthropic_capability_catalog::{
    CatalogSources, DropParams, EffortThresholds,
};
use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::Step;
use super::fixture::NoHfTransport;
use super::warnings::Warnings;
use crate::helpers::{HelperError, OrHelperError as _};

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
            Step::ProviderQualifiedCatalog {
                model,
                custom_llm_provider,
            } => Ok(self
                .provider_qualified_catalog(model, custom_llm_provider, environment)
                .await),
            Step::AdaptiveThinking {
                model,
                custom_llm_provider,
                params,
            } => {
                self.adaptive_thinking(model, custom_llm_provider, params, environment)
                    .await
            }
            Step::ForcedToolDowngrade {
                model,
                tool_choice,
                drop_params,
                global_drop_params,
            } => Ok(self.forced_tool_downgrade(
                model,
                tool_choice,
                DropParams {
                    global: *global_drop_params,
                    call: *drop_params,
                },
            )),
            Step::SamplingCapability {
                model,
                param,
                value,
                output_key,
                drop_params,
                global_drop_params,
            } => {
                let request = SamplingRequest {
                    model,
                    param: *param,
                    value,
                    drop: DropParams {
                        global: *global_drop_params,
                        call: *drop_params,
                    },
                };
                self.sampling_capability(&request, output_key)
            }
            Step::LegacyBudgetEffort {
                model,
                custom_llm_provider,
                budget_tokens,
            } => Ok(self
                .legacy_budget_effort(model, custom_llm_provider, *budget_tokens, environment)
                .await),
            Step::DisabledThinkingWarning {
                model,
                custom_llm_provider,
                params,
            } => Ok(self
                .disabled_thinking_warning(model, custom_llm_provider, params, environment)
                .await),
            Step::EffortBetaGate {
                model,
                custom_llm_provider,
                params,
            } => Ok(self
                .effort_beta_gate(model, custom_llm_provider, params, environment)
                .await),
        }
    }

    fn forced_tool_downgrade(&self, model: &str, tool_choice: &Json, drop: DropParams) -> Value {
        match apply_forced_tool_choice(model, tool_choice, drop, &self.catalogs) {
            Ok(choice) => json!({
                "tool_choice": choice,
                "warning": self.warnings.first(),
            }),
            Err(error) => json!({"error": {"status": error.status_code()}}),
        }
    }

    fn sampling_capability(
        &self,
        request: &SamplingRequest<'_>,
        output_key: &str,
    ) -> Result<Value, Box<HelperError>> {
        match apply_sampling_param(request, &self.catalogs) {
            Ok(forwarded) => {
                let mut params = IndexMap::new();
                if let Some(value) = forwarded {
                    params.insert(output_key.to_owned(), value);
                }
                serde_json::to_value(&params).or_value_error()
            }
            Err(error) => Ok(json!({"error": {"status": error.status_code()}})),
        }
    }
}
