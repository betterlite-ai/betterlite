use betterlite_server::guardrails::provider_forms::core_tables::core_tables;
use betterlite_server::guardrails::provider_forms::project::{project_model, with_friendly_name};
use betterlite_server::guardrails::provider_forms::spec::{
    FormExtra, FormField, FormModel, FormShape,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

// the oracle case env is empty, so LiteLLM reads DEFAULT_MAX_RECURSE_DEPTH's 100 at import
const MAX_DEPTH: usize = 100;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: String,
}

#[derive(Debug, thiserror::Error)]
enum FormModelError {
    #[error("unknown form model {0}")]
    Unknown(String),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.model.as_str() {
            "synthetic_literal_list" => fields_value(&synthetic_literal_list()),
            "synthetic_nested" => fields_value(&synthetic_nested()),
            "synthetic_generic" => generic_answer(),
            model => table(model),
        }
    }
}

fn table(model: &str) -> HelperResult {
    let tables = core_tables();
    let (_, table) = tables
        .iter()
        .find(|(key, _)| *key == model)
        .ok_or_else(|| FormModelError::Unknown(model.to_owned()))
        .or_value_error()?;
    fields_value(table)
}

fn fields_value(model: &FormModel) -> HelperResult {
    let mut fields = project_model(model, MAX_DEPTH).or_value_error()?;
    with_friendly_name(model, &mut fields);
    Ok(Value::Object(fields))
}

fn generic_answer() -> HelperResult {
    let unfilled = fields_value(&synthetic_generic_unfilled())?;
    let filled = fields_value(&synthetic_generic_filled())?;
    Ok(json!({"unfilled": unfilled, "filled": filled}))
}

fn synthetic_entity() -> FormModel {
    FormModel {
        ui_friendly_name: None,
        fields: vec![
            FormField {
                name: "label",
                description: Some("Entity label shown in the UI"),
                required: true,
                shape: FormShape::Text,
                default: Some(Value::Null),
                extra: FormExtra::NONE,
            },
            FormField {
                name: "score",
                description: Some("Entity score"),
                required: false,
                shape: FormShape::Number,
                default: None,
                extra: FormExtra::NONE,
            },
        ],
    }
}

#[rustfmt::skip]
fn synthetic_literal_list() -> FormModel {
    FormModel { ui_friendly_name: None, fields: vec![
        FormField { name: "kinds", description: Some("Kinds to scan"), required: false, shape: FormShape::List { options: vec![json!("alpha"), json!("beta")] }, default: None, extra: FormExtra::NONE },
        FormField { name: "weights", description: Some("Weight per side"), required: false, shape: FormShape::Mapping { keys: vec![json!("left"), json!("right")], value: Box::new(FormShape::Number) }, default: None, extra: FormExtra::NONE },
        FormField { name: "mode", description: Some("Processing mode"), required: false, shape: FormShape::Choice(vec![json!("fast"), json!("slow")]), default: None, extra: FormExtra::NONE },
        FormField { name: "ratio", description: Some("Share of traffic scanned"), required: false, shape: FormShape::Number, default: None, extra: FormExtra { ui_type: Some("percentage".to_owned()), type_override: None, options: None, min: Some(json!(0.0_f64)), max: Some(json!(1.0_f64)), step: Some(json!(0.05_f64)), default_value: None } },
        FormField { name: "tags", description: Some("Tags attached to a match"), required: false, shape: FormShape::List { options: vec![json!("red")] }, default: None, extra: FormExtra { ui_type: None, type_override: None, options: Some(vec![json!("blue"), json!("green")]), min: None, max: None, step: None, default_value: None } },
        FormField { name: "counts", description: Some("Matches per label"), required: false, shape: FormShape::Mapping { keys: Vec::new(), value: Box::new(FormShape::Number) }, default: None, extra: FormExtra::NONE },
        FormField { name: "enabled", description: Some("Whether the check runs"), required: false, shape: FormShape::Boolean, default: Some(json!(false)), extra: FormExtra::NONE },
        FormField { name: "optional_params", description: Some("Optional parameters for the guardrail"), required: false, shape: FormShape::Other, default: None, extra: FormExtra::NONE },
    ] }
}

fn synthetic_nested() -> FormModel {
    FormModel {
        ui_friendly_name: None,
        fields: vec![
            FormField {
                name: "entity",
                description: Some("Primary entity"),
                required: false,
                shape: FormShape::Nested(Box::new(synthetic_entity())),
                default: None,
                extra: FormExtra::NONE,
            },
            FormField {
                name: "entities",
                description: Some("Entities scanned"),
                required: false,
                shape: FormShape::List {
                    options: Vec::new(),
                },
                default: None,
                extra: FormExtra::NONE,
            },
            FormField {
                name: "optional_params",
                description: Some("Optional parameters for the guardrail"),
                required: false,
                shape: FormShape::Other,
                default: None,
                extra: FormExtra::NONE,
            },
        ],
    }
}

fn synthetic_generic_unfilled() -> FormModel {
    FormModel {
        ui_friendly_name: None,
        fields: vec![FormField {
            name: "enabled",
            description: Some("Whether the check runs"),
            required: false,
            shape: FormShape::Boolean,
            default: None,
            extra: FormExtra::NONE,
        }],
    }
}

fn synthetic_generic_filled() -> FormModel {
    FormModel {
        ui_friendly_name: None,
        fields: vec![
            FormField {
                name: "enabled",
                description: Some("Whether the check runs"),
                required: false,
                shape: FormShape::Boolean,
                default: None,
                extra: FormExtra::NONE,
            },
            FormField {
                name: "optional_params",
                description: Some("Optional parameters for the guardrail"),
                required: false,
                shape: FormShape::Other,
                default: None,
                extra: FormExtra::NONE,
            },
        ],
    }
}
