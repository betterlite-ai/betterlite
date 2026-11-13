use std::collections::BTreeMap;

use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_prompts::arguments::json::Object;
use betterlite_vertex::gemini_format::response::apply_response_format;
use betterlite_vertex::gemini_thinking::audio::map_response_modalities;
use betterlite_vertex::gemini_thinking::audio::{AudioParam, map_audio_params};
use betterlite_vertex::gemini_thinking::effort::{map_effort_to_budget, map_effort_to_level};
use betterlite_vertex::gemini_thinking::param::{
    ThinkingParam, map_thinking_param, validate_thinking_conflicts,
};
use betterlite_vertex::gemini_thinking::server_flags::apply_server_tool_flags;
use betterlite_vertex::gemini_thinking::settings::{MinimalEffortSupport, ThinkingSettings};
use betterlite_vertex::model_urls::model_name::is_gemini_3_or_newer;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    settings: ThinkingSettings,
    #[serde(default)]
    catalog: Catalog,
    steps: Vec<Step>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    ReasoningEffort,
    Thinking,
    Format,
    Modalities,
    Audio,
    ServerFlags,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    op: Operation,
    effort: Option<String>,
    thinking: Option<ThinkingParam>,
    model: Option<String>,
    #[serde(default)]
    optional_params: Object,
    #[serde(default)]
    response_format: Object,
    #[serde(default)]
    modalities: Vec<String>,
    audio: Option<AudioParam>,
    #[serde(default)]
    non_default_params: Object,
}

#[derive(Debug, Default, Deserialize)]
struct Catalog(BTreeMap<String, bool>);

impl MinimalEffortSupport for Catalog {
    fn disables_minimal(&self, model: &str) -> bool {
        self.0.get(model).is_some_and(|supported| !supported)
    }
}

pub(super) fn run(input: &Input) -> Value {
    let results: Vec<Value> = input
        .steps
        .iter()
        .map(|step| match step.op {
            Operation::ReasoningEffort => reasoning_effort(step, input),
            Operation::Thinking => thinking_answer(step, input),
            Operation::Format => format_answer(step),
            Operation::Modalities => json!(map_response_modalities(&step.modalities)),
            Operation::Audio => audio_answer(step),
            Operation::ServerFlags => server_flags(step),
        })
        .collect();
    json!({"results": results})
}

fn reasoning_effort(step: &Step, input: &Input) -> Value {
    if let Err(error) = validate_thinking_conflicts(&step.optional_params, "reasoning_effort") {
        return json!({"error": {"status": error.status()}});
    }
    let effort = step.effort.as_deref().unwrap_or_default();
    let model = step.model.as_deref();
    let mapped = if model.is_some_and(is_gemini_3_or_newer) {
        map_effort_to_level(effort, model, &input.catalog)
    } else {
        map_effort_to_budget(effort, model, &input.settings.budgets)
    };
    match mapped {
        Ok(config) => json!(config),
        Err(error) => json!({"error": {"status": error.status()}}),
    }
}

fn thinking_answer(step: &Step, input: &Input) -> Value {
    if let Err(error) = validate_thinking_conflicts(&step.optional_params, "thinking") {
        return json!({"error": {"status": error.status()}});
    }
    let Some(param) = &step.thinking else {
        return json!({"error": {"status": null}});
    };
    json!(map_thinking_param(
        param,
        step.model.as_deref(),
        &input.settings,
        &input.catalog
    ))
}

fn format_answer(step: &Step) -> Value {
    let mut mapped = step.optional_params.clone();
    let model = step.model.as_deref().unwrap_or_default();
    match apply_response_format(
        &step.response_format,
        model,
        &mut mapped,
        &SchemaSettings::default(),
    ) {
        Ok(()) => json!(mapped),
        Err(error) => json!({"error": {"status": error.status()}}),
    }
}

fn audio_answer(step: &Step) -> Value {
    let Some(audio) = &step.audio else {
        return json!({"error": {"status": null}});
    };
    map_audio_params(audio).map_or_else(
        |error| json!({"error": {"status": error.status()}}),
        |config| json!(config),
    )
}

fn server_flags(step: &Step) -> Value {
    let mut mapped = step.optional_params.clone();
    apply_server_tool_flags(&step.non_default_params, &mut mapped);
    json!(mapped)
}
