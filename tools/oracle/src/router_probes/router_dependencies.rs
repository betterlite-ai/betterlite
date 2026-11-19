use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_server::router_dependencies::config_write::validate_config_write;
use betterlite_server::router_dependencies::dependencies::strategy_router_dependencies;
use betterlite_server::router_dependencies::predicates::{
    carries_complexity_settings, defines_custom_prompt, defines_custom_tiers,
    replaces_tiers_or_prompt, uses_heuristic_v2_classifier,
};
use betterlite_server::router_dependencies::validation::{
    validate_config_placement, validate_model_write,
};
use betterlite_server::router_dependencies::{StrategyRouterKind, classify_strategy_router_model};
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

// the enum in mod.rs would otherwise carry every field of this payload inline in its largest variant
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(Box<Payload>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    check: Check,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    litellm_params: JsonObject,
    #[serde(default)]
    present_fields: Vec<String>,
    #[serde(default)]
    complexity_router_config: Option<JsonObject>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Check {
    Classify,
    Dependencies,
    Predicates,
    Carries,
    ModelWrite,
    Placement,
    ConfigWrite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Output(Box<Fields>);

// every check fills its own fields and leaves the rest empty, and both sides publish them all
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fields {
    check: Check,
    kind: Option<StrategyRouterKind>,
    dependencies: Vec<Dependency>,
    heuristic_v2: Option<bool>,
    custom_tiers: Option<bool>,
    custom_prompt: Option<bool>,
    custom_tier_or_prompt: Option<bool>,
    carries: Option<bool>,
    model_write_error: Option<String>,
    placement_error: Option<String>,
    config_write_accepted: Option<bool>,
}

impl Fields {
    fn blank(check: Check) -> Self {
        Self {
            check,
            kind: None,
            dependencies: Vec::new(),
            heuristic_v2: None,
            custom_tiers: None,
            custom_prompt: None,
            custom_tier_or_prompt: None,
            carries: None,
            model_write_error: None,
            placement_error: None,
            config_write_accepted: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Dependency {
    model_name: String,
    role: String,
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let payload = &input.0;
    let mut fields = Fields::blank(payload.check);
    match payload.check {
        Check::Classify => {
            fields.kind =
                classify_strategy_router_model(payload.model.as_deref().unwrap_or_default());
        }
        Check::Dependencies => fields.dependencies = listed_dependencies(payload),
        Check::Predicates => {
            let config = router_config(&payload.litellm_params);
            fields.heuristic_v2 = Some(uses_heuristic_v2_classifier(config));
            fields.custom_tiers = Some(defines_custom_tiers(config));
            fields.custom_prompt = Some(defines_custom_prompt(config));
            fields.custom_tier_or_prompt = Some(replaces_tiers_or_prompt(config));
        }
        Check::Carries => {
            let present: BTreeSet<String> = payload.present_fields.iter().cloned().collect();
            let model = payload.model.as_deref();
            fields.carries = Some(carries_complexity_settings(model, &present));
        }
        Check::ModelWrite | Check::Placement | Check::ConfigWrite => {
            write_fields(payload, &mut fields);
        }
    }
    Ok(ProbeOutput::RouterDependencies(Output(Box::new(fields))))
}

fn listed_dependencies(payload: &Payload) -> Vec<Dependency> {
    let mut listed: Vec<Dependency> = strategy_router_dependencies(&payload.litellm_params)
        .into_iter()
        .map(|dependency| Dependency {
            model_name: dependency.model_name,
            role: dependency.role.name().to_owned(),
        })
        .collect();
    // both sides sort by role name then model name, so the list's order is the contract's
    listed.sort_unstable_by(|first, second| {
        first
            .role
            .cmp(&second.role)
            .then_with(|| first.model_name.cmp(&second.model_name))
    });
    listed
}

fn write_fields(payload: &Payload, fields: &mut Fields) {
    match payload.check {
        Check::ModelWrite => {
            let present: BTreeSet<String> = payload.present_fields.iter().cloned().collect();
            let model = payload.model.as_deref().unwrap_or_default();
            fields.model_write_error = validate_model_write(model, &present)
                .err()
                .map(|error| error.to_string());
        }
        Check::Placement => {
            fields.placement_error = validate_config_placement(Some(&payload.litellm_params))
                .err()
                .map(|error| error.to_string());
        }
        Check::ConfigWrite => {
            let declared = payload
                .complexity_router_config
                .as_ref()
                .map(|config| Json::Object(config.clone()));
            fields.config_write_accepted = Some(validate_config_write(declared.as_ref()).is_ok());
        }
        Check::Classify | Check::Dependencies | Check::Predicates | Check::Carries => {}
    }
}

// LiteLLM reads the value under the key only when it is a mapping, so a non-object config reads as absent
fn router_config(litellm_params: &JsonObject) -> Option<&JsonObject> {
    match litellm_params.get("complexity_router_config") {
        Some(Json::Object(fields)) => Some(fields),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    }
}
