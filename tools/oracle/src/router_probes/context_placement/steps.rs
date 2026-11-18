use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_config::sections::context_placement as section;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::chat::{ChatMessage, MessageContent, Role};
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::complexity_scoring::tier::{ComplexityTier, Tier};
use betterlite_router::context_placement::ContextPlacement;
use betterlite_router::context_placement::error::PlacementError;
use betterlite_router::context_placement::facts::{ModelLimitKey, RouterModelInfo};
use betterlite_router::context_placement::placement::{PlacementRequest, apply_context_placement};
use betterlite_router::context_placement::request::{
    OutOfBand, RequestCounter, RequestFacts, out_of_band_text, request_byte_upper_bound,
};
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::tier_definitions::model::{TierModelEntry, TierPool, TierSet};
use indexmap::IndexMap;

use super::report::{applied_output, ceilings, facts, placement_output};
use super::wire::{ConfigInput, DeploymentInput, GroupInput, Input, Output};
use crate::router_probes::error::OperationError;

// the probe answers get_router_model_info from the deployment's own probe_router entry
struct ProbeModelInfo;

impl RouterModelInfo for ProbeModelInfo {
    fn limit(&self, _group: &str, deployment: &DeploymentSpec, key: ModelLimitKey) -> Option<u64> {
        let Some(Json::Object(info)) = deployment.extra.get("probe_router") else {
            return None;
        };
        match info.get(key.field()) {
            Some(Json::Number(number)) => number.as_u64(),
            _ => None,
        }
    }
}

// a count of None stands for the real counter failing, which leaves the gate without a token count
struct ProbeCounter {
    count: Option<u64>,
}

#[async_trait]
impl RequestCounter for ProbeCounter {
    async fn count(&self, _messages: &[ChatMessage], _text: &str) -> Result<u64, PlacementError> {
        match self.count {
            Some(count) => Ok(count),
            None => Err(PlacementError::Count {
                message: "the probe count is unavailable".to_owned(),
            }),
        }
    }
}

struct Probe<'input> {
    input: &'input Input,
    config: section::Config,
    set: TierSet,
    deployments: DeploymentCatalog,
    model_info: ProbeModelInfo,
    counter: ProbeCounter,
    tier: Tier,
    messages: Vec<ChatMessage>,
    out_of_band: OutOfBand<'input>,
}

impl Probe<'_> {
    fn service(&self) -> ContextPlacement<'_> {
        ContextPlacement {
            config: &self.config,
            deployments: &self.deployments,
            model_info: &self.model_info,
            counter: &self.counter,
        }
    }

    // the fit is computed once and handed to the placement, as the router does per request
    async fn output(&self) -> Result<Output, OperationError> {
        let service = self.service();
        let request = RequestFacts {
            messages: Some(&self.messages),
            out_of_band: self.out_of_band,
            compaction_pending: false,
        };
        let fit = service
            .request_context_fit(&self.set, self.input.default_model.as_deref(), &request)
            .await;
        let placement_request = PlacementRequest {
            tier: &self.tier,
            set: &self.set,
            default_model: self.input.default_model.as_deref(),
            pool_override: self.input.pool_override.as_deref(),
            request,
        };
        let placement = service
            .context_window_placement(&placement_request, Some(&fit))
            .await
            .map_err(|error| OperationError::InvalidRequest {
                message: error.to_string(),
            })?;
        let applied = apply_context_placement(
            self.tier.clone(),
            self.input.signals.clone(),
            placement.as_ref(),
        );
        Ok(Output {
            out_of_band: out_of_band_text(&self.out_of_band),
            byte_bound: request_byte_upper_bound(Some(&self.messages), &self.out_of_band),
            facts: facts(&fit),
            needed: fit.needed,
            ceilings: ceilings(&service, &self.input.groups),
            placement: placement.map(placement_output),
            applied: applied_output(applied),
        })
    }
}

fn config(input: Option<&ConfigInput>) -> Result<section::Config, OperationError> {
    // a case without a config leaves the gate off, as the adapter configures it
    let enabled = input.is_some_and(|config| config.enabled);
    let buffer = match input.and_then(|config| config.buffer.as_deref()) {
        Some(text) => text
            .parse::<f64>()
            .map_err(|_| OperationError::InvalidRequest {
                message: format!("the buffer {text} is not a number"),
            })?,
        None => section::DEFAULT_BUFFER,
    };
    Ok(section::Config {
        enable_context_window_escalation: enabled,
        context_window_escalation_buffer: buffer,
    })
}

fn tier(name: &str) -> Tier {
    match ComplexityTier::built_in(name) {
        Some(built_in) => Tier::BuiltIn(built_in),
        None => Tier::Named(name.to_owned()),
    }
}

fn tier_set(pools: &IndexMap<String, Vec<String>>) -> TierSet {
    TierSet {
        definitions: None,
        pools: pools
            .iter()
            .map(|(name, models)| {
                (
                    name.clone(),
                    TierPool::Pool(
                        models
                            .iter()
                            .map(|model| TierModelEntry {
                                model_name: model.clone(),
                                params: IndexMap::new(),
                            })
                            .collect(),
                    ),
                )
            })
            .collect(),
        labels: BTreeMap::new(),
        non_reasoning: false,
        fallback: None,
    }
}

fn deployments(groups: &[GroupInput]) -> DeploymentCatalog {
    let mut catalog = DeploymentCatalog::default();
    for group in groups {
        for deployment in &group.deployments {
            catalog.push(spec(group, deployment));
        }
    }
    catalog
}

fn spec(group: &GroupInput, deployment: &DeploymentInput) -> DeploymentSpec {
    let mut params = JsonObject::new();
    if let Some(model) = &deployment.model {
        params.insert("model".to_owned(), Json::Text(model.clone()));
    }
    if let Some(provider) = &deployment.custom_llm_provider {
        params.insert(
            "custom_llm_provider".to_owned(),
            Json::Text(provider.clone()),
        );
    }
    let mut model_info = JsonObject::new();
    if let Some(limit) = deployment.max_input_tokens {
        model_info.insert("max_input_tokens".to_owned(), number(limit));
    }
    if let Some(limit) = deployment.max_output_tokens {
        model_info.insert("max_output_tokens".to_owned(), number(limit));
    }
    let mut router = JsonObject::new();
    if let Some(limit) = deployment.router_max_input_tokens {
        router.insert("max_input_tokens".to_owned(), number(limit));
    }
    if let Some(limit) = deployment.router_max_output_tokens {
        router.insert("max_output_tokens".to_owned(), number(limit));
    }
    let mut extra = JsonObject::new();
    extra.insert("probe_router".to_owned(), Json::Object(router));
    DeploymentSpec {
        model_name: group.model.clone(),
        litellm_params: params,
        model_info,
        extra,
    }
}

fn number(value: impl Into<serde_json::Number>) -> Json {
    Json::Number(value.into())
}

fn messages(texts: &[String]) -> Vec<ChatMessage> {
    texts
        .iter()
        .map(|text| ChatMessage {
            role: Role::User,
            content: Some(MessageContent::Text(text.clone())),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            passthrough: IndexMap::new(),
        })
        .collect()
}

fn out_of_band(input: &Input) -> OutOfBand<'_> {
    OutOfBand {
        instructions: input.instructions.as_ref(),
        system: input.system.as_ref(),
        ..OutOfBand::default()
    }
}

fn probe(input: &Input) -> Result<Probe<'_>, OperationError> {
    Ok(Probe {
        input,
        config: config(input.config.as_ref())?,
        set: tier_set(&input.pools),
        deployments: deployments(&input.groups),
        model_info: ProbeModelInfo,
        counter: ProbeCounter { count: input.count },
        tier: tier(&input.tier),
        messages: messages(&input.messages),
        out_of_band: out_of_band(input),
    })
}

pub(super) async fn output(input: &Input) -> Result<Output, OperationError> {
    probe(input)?.output().await
}
