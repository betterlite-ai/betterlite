use betterlite_anthropic::anthropic_capability_catalog::thinking::{
    ThinkingRequest, translate_legacy_thinking,
};
use betterlite_anthropic::anthropic_capability_catalog::{EffortThresholds, OptionalParams};
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::arguments::json::Json as PromptJson;
use indexmap::IndexMap;

use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::claude_invoke::CUSTOM_LLM_PROVIDER;
use crate::claude_invoke::params::{MapParamsError, MapParamsRequest};
use crate::message_options::OptionsSources;
use crate::message_options::effort::clamp_adaptive_effort;
use crate::model_routes::{ParamMap, ParamValue};

const OUTPUT_CONFIG: &str = "output_config";
const REASONING_EFFORT: &str = "reasoning_effort";
const THINKING: &str = "thinking";

pub(super) fn thinking_param(
    non_default_params: &IndexMap<String, Json>,
) -> Result<Option<PromptJson>, MapParamsError> {
    let Some(value) = non_default_params.get(THINKING) else {
        return Ok(None);
    };
    let converted = serde_json::from_value(serde_json::to_value(value)?)?;
    Ok(Some(converted))
}

impl<'request, Lookup> MapParamsRequest<'request, Lookup>
where
    Lookup: ModelInfoLookup,
{
    // LiteLLM rewrites the caller's own map; the clamped copy is what the parent mapping reads here.
    pub(super) async fn clamp_reasoning_effort<Source, Transport>(
        &self,
        params: &mut IndexMap<String, Json>,
        environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    ) -> Result<(), MapParamsError>
    where
        Source: DynamicMetadataSource,
        Transport: ModelInfoClient,
    {
        let Some(mut clamped) = effort_only(params) else {
            return Ok(());
        };
        let mut sources = OptionsSources {
            environment: &mut *environment,
            catalogs: self.catalogs,
            budgets: self.budgets,
        };
        clamp_adaptive_effort(
            self.model,
            &mut clamped,
            &mut sources,
            self.lookup,
            self.packaged,
        )
        .await?;
        write_effort(params, &clamped);
        Ok(())
    }

    pub(super) async fn translate_thinking<Source, Transport>(
        &self,
        optional_params: &mut IndexMap<String, Json>,
        environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    ) -> Result<(), MapParamsError>
    where
        Source: DynamicMetadataSource,
        Transport: ModelInfoClient,
    {
        translated(self, optional_params, environment).await
    }
}

async fn translated<Source, Transport, Lookup>(
    request: &MapParamsRequest<'_, Lookup>,
    optional_params: &mut IndexMap<String, Json>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<(), MapParamsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    let thinking_request = translation_request(request.model, request.budgets);
    let mut params = OptionalParams {
        reasoning_effort: optional_params.get(REASONING_EFFORT).cloned(),
        thinking: optional_params.get(THINKING).cloned(),
        output_config: optional_params.get(OUTPUT_CONFIG).cloned(),
    };
    let before = params.clone();
    translate_legacy_thinking(
        &thinking_request,
        &mut params,
        environment,
        request.catalogs,
    )
    .await?;
    write_translated(optional_params, &before, &params);
    Ok(())
}

const fn translation_request<'model>(
    model: &'model str,
    budgets: &Settings,
) -> ThinkingRequest<'model> {
    ThinkingRequest {
        model,
        custom_llm_provider: CUSTOM_LLM_PROVIDER,
        thresholds: EffortThresholds {
            medium: budgets.medium,
            high: budgets.high,
            xhigh: budgets.xhigh,
        },
    }
}

fn effort_only(params: &IndexMap<String, Json>) -> Option<ParamMap> {
    let Json::Text(effort) = params.get(REASONING_EFFORT)? else {
        return None;
    };
    let mut clamped = ParamMap::new();
    clamped.insert(
        REASONING_EFFORT.to_owned(),
        ParamValue::Text(effort.clone()),
    );
    Some(clamped)
}

fn write_effort(params: &mut IndexMap<String, Json>, clamped: &ParamMap) {
    if let Some(ParamValue::Text(effort)) = clamped.get(REASONING_EFFORT) {
        params.insert(REASONING_EFFORT.to_owned(), Json::Text(effort.clone()));
    }
}

fn write_translated(
    optional_params: &mut IndexMap<String, Json>,
    before: &OptionalParams,
    after: &OptionalParams,
) {
    write_back(
        optional_params,
        THINKING,
        before.thinking.as_ref(),
        after.thinking.clone(),
    );
    write_back(
        optional_params,
        OUTPUT_CONFIG,
        before.output_config.as_ref(),
        after.output_config.clone(),
    );
}

// B568 changes only the entries it rewrote, so an untouched map keeps the caller's order.
fn write_back(
    optional_params: &mut IndexMap<String, Json>,
    key: &str,
    before: Option<&Json>,
    after: Option<Json>,
) {
    if before == after.as_ref() {
        return;
    }
    match after {
        Some(value) => {
            optional_params.insert(key.to_owned(), value);
        }
        None => {
            optional_params.shift_remove(key);
        }
    }
}

pub(super) fn apply_display_updates(requested: bool, optional_params: &mut IndexMap<String, Json>) {
    if !requested {
        return;
    }
    let Some(Json::Object(thinking)) = optional_params.get(THINKING) else {
        return;
    };
    let adaptive = matches!(thinking.get("type"), Some(Json::Text(kind)) if kind == "adaptive");
    if !adaptive {
        return;
    }
    let mut rewritten = IndexMap::new();
    rewritten.insert("type".to_owned(), Json::Text("adaptive".to_owned()));
    rewritten.insert("display".to_owned(), Json::Text("updates".to_owned()));
    optional_params.insert(THINKING.to_owned(), Json::Object(rewritten));
}
