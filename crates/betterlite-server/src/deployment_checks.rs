use std::collections::BTreeSet;

use betterlite_config::agentic_loop_ceiling::{
    AgenticLoopCeilingError, AgenticLoopValue, validated_max_agentic_loops,
};
use betterlite_logging::analytics::python;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::access_windows::window::access_windows_config_error;
use betterlite_router::deployment_identity::stable_id::generate_model_id;
use jiff::tz::TimeZoneDatabase;
use serde_json::Number;
use thiserror::Error;

use crate::router_dependencies::error::ComplexityPlacementError;
use crate::router_dependencies::predicates::{
    carries_complexity_settings, replaces_tiers_or_prompt, uses_heuristic_v2_classifier,
};
use crate::router_dependencies::validation::validate_config_placement;
use crate::router_dependencies::{STRATEGY_ROUTER_PARAM_FIELDS, is_complexity_router_model};

mod tests;

// The license refusal every capability message ends with, as litellm/proxy/auth/litellm_license.py spells it.
pub const AUTO_ROUTER_LICENSE_REMEDY: &str =
    "A LiteLLM license with the 'auto_router' feature lifts the limit.";

#[derive(Debug, Error)]
pub enum DeploymentCheckError {
    #[error("{field} must be an integer, got {literal}")]
    NotInteger { field: String, literal: String },
    #[error("{field} must be at least 1, got {value}")]
    BelowMinimum { field: String, value: i64 },
    #[error("model {model_name}: {source}")]
    Misplaced {
        model_name: String,
        #[source]
        source: ComplexityPlacementError,
    },
    #[error("{message}")]
    AccessWindows { message: String },
    #[error("{message}")]
    CapabilityLimits { message: String },
    #[error("the rejected value could not be rendered as Python text")]
    Render(#[from] python::FormatError),
}

// A per-deployment ceiling the agentic loop cannot honor would otherwise be dropped or fail every request.
pub fn check_max_agentic_loops(model: &JsonObject) -> Result<(), DeploymentCheckError> {
    let Some(params) = object(model.get("litellm_params")) else {
        return Ok(());
    };
    let Some(value) = params.get("max_agentic_loops") else {
        return Ok(());
    };
    let field = format!(
        "litellm_params.max_agentic_loops on model {}",
        python::repr(&reported_name(model))?
    );
    let parsed = match value {
        Json::List(_) | Json::Object(_) => {
            return Err(DeploymentCheckError::NotInteger {
                field,
                literal: python::repr(value)?,
            });
        }
        Json::Null => AgenticLoopValue::Null,
        Json::Bool(flag) => AgenticLoopValue::Boolean(*flag),
        Json::Number(printed) => number_loop_value(printed),
        Json::Text(written) => AgenticLoopValue::Text(written.clone()),
    };
    match validated_max_agentic_loops(&parsed, &field) {
        Ok(_) => Ok(()),
        Err(AgenticLoopCeilingError::NotWholeNumber { field: rejected }) => {
            Err(DeploymentCheckError::NotInteger {
                field: rejected,
                literal: python::repr(value)?,
            })
        }
        Err(AgenticLoopCeilingError::BelowMinimum {
            field: rejected,
            value: minimum,
        }) => Err(DeploymentCheckError::BelowMinimum {
            field: rejected,
            value: minimum,
        }),
    }
}

// A complexity-router setting one level too high configures nothing and reaches the provider as an unknown param.
pub fn check_complexity_placement(model: &JsonObject) -> Result<(), DeploymentCheckError> {
    let Some(params) = object(model.get("litellm_params")) else {
        return Ok(());
    };
    let mut present_fields = BTreeSet::new();
    for field in STRATEGY_ROUTER_PARAM_FIELDS {
        if params
            .get(field)
            .is_some_and(|value| !matches!(value, Json::Null))
        {
            present_fields.insert(field.to_owned());
        }
    }
    if !carries_complexity_settings(text(params.get("model")), &present_fields) {
        return Ok(());
    }
    if let Err(source) = validate_config_placement(Some(params)) {
        return Err(DeploymentCheckError::Misplaced {
            model_name: python::repr(&reported_name(model))?,
            source,
        });
    }
    Ok(())
}

// A malformed access window would otherwise drop the deployment at build time instead of refusing startup.
pub fn check_access_windows(
    model: &JsonObject,
    database: &TimeZoneDatabase,
) -> Result<(), DeploymentCheckError> {
    let Some(model_info) = object(model.get("model_info")) else {
        return Ok(());
    };
    let model_name = text(model.get("model_name")).unwrap_or_default();
    access_windows_config_error(model_info, model_name, database).map_or(Ok(()), |message| {
        Err(DeploymentCheckError::AccessWindows { message })
    })
}

// More auto-routers claiming a licensed capability than the license allows would silently lose the extras.
pub fn check_capability_limits(
    model_list: &[JsonObject],
    limit: Option<usize>,
) -> Result<(), DeploymentCheckError> {
    let Some(bound) = limit else {
        return Ok(());
    };
    let mut violations = Vec::new();
    for capability in GatedCapability::ALL {
        let held = model_list
            .iter()
            .filter(|entry| claimed_capability_of(entry) == Some(capability))
            .count();
        if held > bound {
            let wording = capability.wording();
            violations.push(format!(
                "At most {bound} auto-router(s) {} can be registered but this would make {held}. {}",
                wording.subject, wording.remedy
            ));
        }
    }
    if violations.is_empty() {
        return Ok(());
    }
    Err(DeploymentCheckError::CapabilityLimits {
        message: format!(
            "config.yaml model_list: {} {AUTO_ROUTER_LICENSE_REMEDY}",
            violations.join(" ")
        ),
    })
}

// The id is pinned from the raw params, before plugin paths are swapped, so the reconcile re-hashes find it.
pub fn pin_complexity_model_id(model: &mut JsonObject) {
    let Some(generated) = generated_complexity_id(model) else {
        return;
    };
    let model_info = model
        .entry("model_info".to_owned())
        .or_insert(Json::Object(JsonObject::new()));
    if !matches!(model_info, Json::Object(_)) {
        *model_info = Json::Object(JsonObject::new());
    }
    let Json::Object(fields) = model_info else {
        return;
    };
    if matches!(fields.get("id"), None | Some(Json::Null)) {
        fields.insert("id".to_owned(), Json::Text(generated));
    }
}

fn generated_complexity_id(model: &JsonObject) -> Option<String> {
    let params = object(model.get("litellm_params"))?;
    if !matches!(
        params.get("complexity_router_config"),
        Some(Json::Object(_))
    ) {
        return None;
    }
    Some(generate_model_id(
        text(model.get("model_name")).unwrap_or_default(),
        params,
    ))
}

fn claimed_capability_of(entry: &JsonObject) -> Option<GatedCapability> {
    let params = object(entry.get("litellm_params"))?;
    if !is_complexity_router_model(text(params.get("model"))) {
        return None;
    }
    let config = object(params.get("complexity_router_config"));
    GatedCapability::ALL
        .into_iter()
        .find(|capability| capability.claims(config))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GatedCapability {
    HeuristicV2,
    Capability,
    LlmV2,
    Customization,
}

struct Wording {
    subject: &'static str,
    remedy: &'static str,
}

impl GatedCapability {
    const ALL: [Self; 4] = [
        Self::HeuristicV2,
        Self::Capability,
        Self::LlmV2,
        Self::Customization,
    ];

    const fn wording(self) -> Wording {
        match self {
            Self::HeuristicV2 => Wording {
                subject: "with classifier_type 'heuristic_v2'",
                remedy: "Use classifier_type 'heuristic' for this router or remove an existing heuristic_v2 router.",
            },
            Self::Capability => Wording {
                subject: "with classifier_type 'capability' (Capability)",
                remedy: "Use a different classifier or remove an existing Capability router.",
            },
            Self::LlmV2 => Wording {
                subject: "with classifier_type 'llm_v2' (Fuse v2)",
                remedy: "Use a different classifier or remove an existing Fuse v2 router.",
            },
            Self::Customization => Wording {
                subject: "with operator-defined tier_definitions or an operator-written classifier prompt",
                remedy: "Use the shipped tiers and classifier prompt for this router or remove an existing router with tier_definitions or its own classifier prompt.",
            },
        }
    }

    fn claims(self, config: Option<&JsonObject>) -> bool {
        match self {
            Self::HeuristicV2 => uses_heuristic_v2_classifier(config),
            Self::Capability => matches!(
                config.and_then(|fields| fields.get("classifier_type")),
                Some(Json::Text(name)) if name == "capability"
            ),
            Self::LlmV2 => matches!(
                config.and_then(|fields| fields.get("classifier_type")),
                Some(Json::Text(name)) if name == "llm_v2"
            ),
            Self::Customization => replaces_tiers_or_prompt(config),
        }
    }
}

fn number_loop_value(number: &Number) -> AgenticLoopValue {
    let printed = number.to_string();
    if printed.contains(['.', 'e', 'E']) {
        return printed
            .parse::<f64>()
            .map_or(AgenticLoopValue::Text(printed), AgenticLoopValue::Float);
    }
    printed
        .parse::<i64>()
        .map_or(AgenticLoopValue::Text(printed), AgenticLoopValue::Integer)
}

// the raw model_name a message reports, as LiteLLM's model.get("model_name", "") hands it over
fn reported_name(model: &JsonObject) -> Json {
    model
        .get("model_name")
        .cloned()
        .unwrap_or(Json::Text(String::new()))
}

const fn object(value: Option<&Json>) -> Option<&JsonObject> {
    match value {
        Some(Json::Object(fields)) => Some(fields),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    }
}

fn text(value: Option<&Json>) -> Option<&str> {
    match value {
        Some(Json::Text(written)) => Some(written),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}
