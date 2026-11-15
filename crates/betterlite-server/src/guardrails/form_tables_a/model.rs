use serde_json::Value;

use crate::guardrails::provider_forms::spec::{FormExtra, FormField, FormModel, FormShape};

#[derive(Debug)]
pub(crate) struct Model {
    pub(crate) name: Option<&'static str>,
    pub(crate) fields: &'static [Field],
}

#[derive(Debug)]
pub(crate) struct Field {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) required: bool,
    pub(crate) shape: Shape,
    pub(crate) default: Initial,
    pub(crate) extra: Extra,
}

#[derive(Debug)]
pub(crate) enum Shape {
    Text,
    Number,
    Boolean,
    Choice(&'static [&'static str]),
    List(&'static [&'static str]),
    Mapping(&'static [&'static str], &'static Shape),
    Nested(&'static Model),
    RawObject,
    Other,
}

#[derive(Debug)]
pub(crate) enum Initial {
    None,
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(&'static str),
}

#[derive(Debug)]
pub(crate) struct Extra {
    pub(crate) ui_type: Option<&'static str>,
    pub(crate) options: Option<&'static [&'static str]>,
    pub(crate) min: Option<f64>,
    pub(crate) max: Option<f64>,
    pub(crate) step: Option<f64>,
    pub(crate) default_value: Option<f64>,
}

impl Extra {
    pub(crate) const NONE: Self = Self {
        ui_type: None,
        options: None,
        min: None,
        max: None,
        step: None,
        default_value: None,
    };
}

#[must_use]
pub(crate) fn build_model(model: &Model) -> FormModel {
    FormModel {
        ui_friendly_name: model.name,
        fields: model.fields.iter().map(build_field).collect(),
    }
}

fn build_field(field: &Field) -> FormField {
    FormField {
        name: field.name,
        description: Some(field.description),
        required: field.required,
        shape: build_shape(&field.shape),
        default: build_default(&field.default),
        extra: FormExtra {
            ui_type: field.extra.ui_type.map(str::to_owned),
            type_override: None,
            options: field.extra.options.map(strings),
            min: field.extra.min.map(Value::from),
            max: field.extra.max.map(Value::from),
            step: field.extra.step.map(Value::from),
            default_value: field.extra.default_value.map(Value::from),
        },
    }
}

fn build_shape(shape: &Shape) -> FormShape {
    match shape {
        Shape::Text => FormShape::Text,
        Shape::Number => FormShape::Number,
        Shape::Boolean => FormShape::Boolean,
        Shape::RawObject => FormShape::RawObject,
        Shape::Other => FormShape::Other,
        Shape::Choice(options) => FormShape::Choice(strings(options)),
        Shape::List(options) => FormShape::List {
            options: strings(options),
        },
        Shape::Mapping(keys, value) => FormShape::Mapping {
            keys: strings(keys),
            value: Box::new(build_shape(value)),
        },
        Shape::Nested(model) => FormShape::Nested(Box::new(build_model(model))),
    }
}

fn build_default(default: &Initial) -> Option<Value> {
    match default {
        Initial::None => None,
        Initial::Null => Some(Value::Null),
        Initial::Bool(flag) => Some(Value::Bool(*flag)),
        Initial::Int(number) => Some(Value::from(*number)),
        Initial::Float(number) => Some(Value::from(*number)),
        Initial::Text(text) => Some(Value::from(*text)),
    }
}

fn strings(items: &[&str]) -> Vec<Value> {
    items.iter().copied().map(Value::from).collect()
}
