use std::collections::BTreeMap;

use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_prompts::arguments::json::Object;
use betterlite_vertex::gemini_format::response::apply_response_format;
use betterlite_vertex::gemini_thinking::audio::{AudioParam, map_audio_params};
use betterlite_vertex::gemini_thinking::effort::{map_effort_to_budget, map_effort_to_level};
use betterlite_vertex::gemini_thinking::modalities::map_response_modalities;
use betterlite_vertex::gemini_thinking::param::{
    ThinkingParam, map_thinking_param, validate_thinking_conflicts,
};
use betterlite_vertex::gemini_thinking::server_flags::apply_server_tool_flags;
use betterlite_vertex::gemini_thinking::settings::{
    MinimalEffortSupport, ThinkingBudgets, ThinkingSettings,
};
use betterlite_vertex::model_urls::model_name::is_gemini_3_or_newer;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    settings: InputSettings,
    #[serde(default)]
    catalog: BTreeMap<String, bool>,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputSettings {
    budgets: InputBudgets,
    default_thinking_level_low: bool,
    max_recurse_depth: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputBudgets {
    minimal: i64,
    minimal_flash: i64,
    minimal_pro: i64,
    minimal_flash_lite: i64,
    low: i64,
    medium: i64,
    high: i64,
    disable: i64,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Step {
    ReasoningEffort {
        effort: String,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        optional_params: Object,
    },
    Thinking {
        thinking: ThinkingParam,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        optional_params: Object,
    },
    Format {
        response_format: Object,
        model: String,
        #[serde(default)]
        optional_params: Object,
    },
    Modalities {
        modalities: Vec<String>,
    },
    Audio {
        audio: AudioParam,
    },
    ServerFlags {
        #[serde(default)]
        non_default_params: Object,
        #[serde(default)]
        optional_params: Object,
    },
}

struct Fixture<'catalog> {
    settings: ThinkingSettings,
    schema: SchemaSettings,
    catalog: Catalog<'catalog>,
}

struct Catalog<'entries>(&'entries BTreeMap<String, bool>);

impl MinimalEffortSupport for Catalog<'_> {
    fn disables_minimal(&self, model: &str) -> bool {
        self.0.get(model).is_some_and(|supported| !supported)
    }
}

pub(super) fn run(input: &Input) -> Value {
    let fixture = Fixture {
        settings: ThinkingSettings {
            budgets: ThinkingBudgets {
                minimal: input.settings.budgets.minimal,
                minimal_flash: input.settings.budgets.minimal_flash,
                minimal_pro: input.settings.budgets.minimal_pro,
                minimal_flash_lite: input.settings.budgets.minimal_flash_lite,
                low: input.settings.budgets.low,
                medium: input.settings.budgets.medium,
                high: input.settings.budgets.high,
                disable: input.settings.budgets.disable,
            },
            default_thinking_level_low: input.settings.default_thinking_level_low,
        },
        schema: SchemaSettings {
            max_recurse_depth: input.settings.max_recurse_depth,
        },
        catalog: Catalog(&input.catalog),
    };
    let results: Vec<Value> = input
        .steps
        .iter()
        .map(|step| answer(step, &fixture))
        .collect();
    json!({"results": results})
}

fn answer(step: &Step, fixture: &Fixture<'_>) -> Value {
    match step {
        Step::ReasoningEffort {
            effort,
            model,
            optional_params,
        } => reasoning_effort(effort, model.as_deref(), optional_params, fixture),
        Step::Thinking {
            thinking,
            model,
            optional_params,
        } => thinking_answer(thinking, model.as_deref(), optional_params, fixture),
        Step::Format {
            response_format,
            model,
            optional_params,
        } => format_answer(response_format, model, optional_params, fixture),
        Step::Modalities { modalities } => json!(map_response_modalities(modalities)),
        Step::Audio { audio } => match map_audio_params(audio) {
            Ok(config) => json!(config),
            Err(error) => json!({"error": {"status": error.status()}}),
        },
        Step::ServerFlags {
            non_default_params,
            optional_params,
        } => {
            let mut mapped = optional_params.clone();
            apply_server_tool_flags(non_default_params, &mut mapped);
            json!(mapped)
        }
    }
}

fn reasoning_effort(
    effort: &str,
    model: Option<&str>,
    optional_params: &Object,
    fixture: &Fixture<'_>,
) -> Value {
    if let Err(error) = validate_thinking_conflicts(optional_params, "reasoning_effort") {
        return json!({"error": {"status": error.status()}});
    }
    let mapped = if model.is_some_and(is_gemini_3_or_newer) {
        map_effort_to_level(effort, model, &fixture.catalog)
    } else {
        map_effort_to_budget(effort, model, &fixture.settings.budgets)
    };
    match mapped {
        Ok(config) => json!(config),
        Err(error) => json!({"error": {"status": error.status()}}),
    }
}

fn thinking_answer(
    param: &ThinkingParam,
    model: Option<&str>,
    optional_params: &Object,
    fixture: &Fixture<'_>,
) -> Value {
    if let Err(error) = validate_thinking_conflicts(optional_params, "thinking") {
        return json!({"error": {"status": error.status()}});
    }
    json!(map_thinking_param(
        param,
        model,
        &fixture.settings,
        &fixture.catalog
    ))
}

fn format_answer(
    response_format: &Object,
    model: &str,
    optional_params: &Object,
    fixture: &Fixture<'_>,
) -> Value {
    let mut mapped = optional_params.clone();
    match apply_response_format(response_format, model, &mut mapped, &fixture.schema) {
        Ok(()) => json!(mapped),
        Err(_schema) => json!({"error": {"status": null}}),
    }
}
