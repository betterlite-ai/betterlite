use std::sync::{Mutex, PoisonError};

use betterlite_core::api::messages_reasoning_route::seams::{
    BridgeOutcome, BridgePolicy, BridgeRequest, ModelMetadata, ProviderLookup, ProviderResolution,
    ReasoningSeams, ResolvedTarget,
};
use betterlite_core::api::messages_reasoning_route::{
    AutoSummary, CompletionKwargs, normalize_effort, plain_effort, route_thinking,
};
use betterlite_core::responses_bridge::ModelInfoError;
use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::record::ModelInfo;
use betterlite_models::model_metadata_resolution::names::ModelCostMap;
use indexmap::IndexMap;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::{BridgeScript, ResolvedScript, helper_error};

pub(super) fn normalize_answer(
    kwargs: &IndexMap<String, Json>,
    model_info: Option<&IndexMap<String, Json>>,
) -> HelperResult {
    let metadata = Metadata {
        info: model_info.map(|fields| ModelInfo {
            fields: fields.clone(),
        }),
        cost: empty_cost()?,
        recorder: Recorder::default(),
    };
    let mut completion = CompletionKwargs {
        fields: kwargs.clone(),
    };
    normalize_effort(&mut completion, &metadata);
    Ok(json!({
        "kwargs": completion.fields,
        "seams": metadata.recorder.into_entries(),
    }))
}

pub(super) fn route_answer(
    kwargs: &IndexMap<String, Json>,
    thinking: &Json,
    model_info: Option<&IndexMap<String, Json>>,
    provider: Option<&ResolvedScript>,
    settings: AutoSummary<'_>,
) -> HelperResult {
    let metadata = Metadata {
        info: model_info.map(|fields| ModelInfo {
            fields: fields.clone(),
        }),
        cost: empty_cost()?,
        recorder: Recorder::default(),
    };
    let resolution = Provider {
        script: provider.cloned(),
        recorder: Recorder::default(),
    };
    let mut completion = CompletionKwargs {
        fields: kwargs.clone(),
    };
    route_thinking(
        &mut completion,
        Some(thinking),
        settings,
        &ReasoningSeams {
            provider: &resolution,
            metadata: &metadata,
            bridge: &NoBridge,
        },
    );
    let mut entries = resolution.recorder.into_entries();
    entries.extend(metadata.recorder.into_entries());
    Ok(json!({
        "kwargs": completion.fields,
        "seams": entries,
    }))
}

pub(super) fn plain_answer(
    kwargs: &IndexMap<String, Json>,
    thinking: &Json,
    provider: Option<&ResolvedScript>,
    bridge: &BridgeScript,
) -> HelperResult {
    let resolution = Provider {
        script: provider.cloned(),
        recorder: Recorder::default(),
    };
    let policy = Bridge {
        script: bridge.clone(),
        recorder: Recorder::default(),
    };
    let metadata = Metadata {
        info: None,
        cost: empty_cost()?,
        recorder: Recorder::default(),
    };
    let completion = CompletionKwargs {
        fields: kwargs.clone(),
    };
    let effort = plain_effort(
        &completion,
        Some(thinking),
        &ReasoningSeams {
            provider: &resolution,
            metadata: &metadata,
            bridge: &policy,
        },
    )
    .map_err(|error| helper_error("BridgeError", None, error.to_string()))?;
    let mut entries = resolution.recorder.into_entries();
    entries.extend(policy.recorder.into_entries());
    entries.extend(metadata.recorder.into_entries());
    Ok(json!({
        "effort": effort,
        "seams": entries,
    }))
}

fn empty_cost() -> Result<ModelCostMap, Box<HelperError>> {
    ModelCostMap::parse(b"{}")
        .map_err(|source| helper_error("InvalidInput", None, source.to_string()))
}

#[derive(Default)]
struct Recorder {
    entries: Mutex<Vec<Value>>,
}

impl Recorder {
    fn record(&self, entry: Value) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }

    fn into_entries(self) -> Vec<Value> {
        self.entries
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

struct Provider {
    script: Option<ResolvedScript>,
    recorder: Recorder,
}

impl ProviderResolution for Provider {
    fn resolve(&self, lookup: &ProviderLookup<'_>) -> Option<ResolvedTarget> {
        self.recorder.record(json!({
            "seam": "provider",
            "args": {
                "model": lookup.model,
                "custom_llm_provider": lookup.custom_llm_provider,
                "api_base": lookup.api_base,
                "api_key": lookup.api_key,
            },
        }));
        self.script.as_ref().map(|script| ResolvedTarget {
            model: script.local.clone(),
            provider: script.provider.clone(),
            api_base: script.api_base.clone(),
        })
    }
}

struct Metadata {
    info: Option<ModelInfo>,
    cost: ModelCostMap,
    recorder: Recorder,
}

impl ModelMetadata for Metadata {
    fn model_info(
        &self,
        model: Option<&str>,
        custom_llm_provider: Option<&str>,
    ) -> Option<ModelInfo> {
        self.recorder.record(json!({
            "seam": "model_info",
            "args": {
                "model": model,
                "custom_llm_provider": custom_llm_provider,
            },
        }));
        self.info.clone()
    }

    fn model_cost(&self) -> &ModelCostMap {
        &self.cost
    }
}

struct Bridge {
    script: BridgeScript,
    recorder: Recorder,
}

impl BridgePolicy for Bridge {
    fn bridge(&self, request: &BridgeRequest<'_>) -> Result<BridgeOutcome, ModelInfoError> {
        self.recorder.record(json!({
            "seam": "bridge",
            "args": {
                "model": request.model,
                "custom_llm_provider": request.custom_llm_provider,
                "web_search_options": request.web_search_options,
                "tools": request.tools,
                "reasoning_effort": request.reasoning_effort,
                "api_base": request.api_base,
            },
        }));
        if self.script.fails {
            return Err(ModelInfoError::Unknown {
                model: request.model.to_owned(),
            });
        }
        Ok(BridgeOutcome {
            mode: self.script.mode.clone(),
            model: request.model.to_owned(),
        })
    }
}

struct NoBridge;

impl BridgePolicy for NoBridge {
    fn bridge(&self, request: &BridgeRequest<'_>) -> Result<BridgeOutcome, ModelInfoError> {
        Err(ModelInfoError::Unknown {
            model: request.model.to_owned(),
        })
    }
}
