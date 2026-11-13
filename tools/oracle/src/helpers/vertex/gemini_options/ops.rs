use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_vertex::gemini_options::assembly::{OptionsContext, map_options};
use betterlite_vertex::gemini_options::auth::map_special_auth_params;
use betterlite_vertex::gemini_options::defaults::{GenerationDefaults, OptionSettings};
use betterlite_vertex::gemini_thinking::settings::{
    MinimalEffortSupport, ThinkingBudgets, ThinkingSettings,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    safety_settings: Option<Json>,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    op: Operation,
    model: Option<String>,
    #[serde(default)]
    non_default_params: Object,
    #[serde(default)]
    optional_params: Object,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Options,
    Auth,
}

struct NoMinimalEffort;

impl MinimalEffortSupport for NoMinimalEffort {
    fn disables_minimal(&self, _model: &str) -> bool {
        false
    }
}

const fn thinking_settings() -> ThinkingSettings {
    ThinkingSettings {
        budgets: ThinkingBudgets {
            minimal: 128,
            minimal_flash: 1,
            minimal_pro: 128,
            minimal_flash_lite: 512,
            low: 1024,
            medium: 2048,
            high: 4096,
            disable: 0,
        },
        default_thinking_level_low: false,
    }
}

pub(super) fn run(input: &Input) -> Value {
    let settings = OptionSettings {
        generation: GenerationDefaults::default(),
        safety_settings: input.safety_settings.clone(),
    };
    let schema = SchemaSettings::default();
    let thinking = thinking_settings();
    let results: Vec<Value> = input
        .steps
        .iter()
        .map(|step| answer(step, &settings, &schema, &thinking))
        .collect();
    json!({ "results": results })
}

fn answer(
    step: &Step,
    settings: &OptionSettings,
    schema: &SchemaSettings,
    thinking: &ThinkingSettings,
) -> Value {
    if matches!(step.op, Operation::Auth) {
        let mapped =
            map_special_auth_params(&step.non_default_params, step.optional_params.clone());
        return match mapped {
            Ok(options) => json!({ "options": Json::Object(options), "warnings": 0 }),
            Err(error) => json!({ "error": { "status": error.status() } }),
        };
    }
    let mut optional = step.optional_params.clone();
    let context = OptionsContext {
        model: step.model.as_deref().unwrap_or_default(),
        non_default: &step.non_default_params,
        settings,
        schema,
        thinking,
        support: &NoMinimalEffort,
    };
    match map_options(&step.non_default_params, &mut optional, &context) {
        Ok(warnings) => json!({ "options": Json::Object(optional), "warnings": warnings }),
        Err(error) => json!({ "error": { "status": error.status() } }),
    }
}
