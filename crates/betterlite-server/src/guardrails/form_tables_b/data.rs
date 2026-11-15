use serde_json::json;

use crate::guardrails::provider_forms::spec::{FormExtra, FormField, FormModel, FormShape};

pub(super) struct SpecModel {
    pub(super) ui_friendly_name: Option<&'static str>,
    pub(super) fields: &'static [SpecField],
}

pub(super) struct SpecField {
    pub(super) name: &'static str,
    pub(super) description: Option<&'static str>,
    pub(super) required: bool,
    pub(super) shape: SpecShape,
    pub(super) default: Option<SpecValue>,
    pub(super) ui_type: Option<&'static str>,
}

pub(super) enum SpecShape {
    Text,
    Number,
    Boolean,
    Mapping {
        keys: &'static [&'static str],
        value: &'static Self,
    },
    List(&'static [&'static str]),
    Choice(&'static [&'static str]),
    Nested(&'static [SpecField]),
    RawObject,
}

pub(super) enum SpecValue {
    Null,
    Bool(bool),
    Integer(i64),
    Float(f64),
    Text(&'static str),
}

pub(super) fn model(spec: &SpecModel) -> FormModel {
    FormModel {
        ui_friendly_name: spec.ui_friendly_name,
        fields: spec.fields.iter().map(field).collect(),
    }
}

fn field(spec: &SpecField) -> FormField {
    FormField {
        name: spec.name,
        description: spec.description,
        required: spec.required,
        shape: shape(&spec.shape),
        default: spec.default.as_ref().map(|value| match value {
            SpecValue::Null => json!(null),
            SpecValue::Bool(flag) => json!(flag),
            SpecValue::Integer(number) => json!(number),
            SpecValue::Float(number) => json!(number),
            SpecValue::Text(text) => json!(text),
        }),
        extra: FormExtra {
            ui_type: spec.ui_type.map(str::to_owned),
            type_override: None,
            options: None,
            min: None,
            max: None,
            step: None,
            default_value: None,
        },
    }
}

fn shape(spec: &SpecShape) -> FormShape {
    match spec {
        SpecShape::Text => FormShape::Text,
        SpecShape::Number => FormShape::Number,
        SpecShape::Boolean => FormShape::Boolean,
        SpecShape::Mapping { keys, value } => FormShape::Mapping {
            keys: keys.iter().map(|text| json!(text)).collect(),
            value: Box::new(shape(value)),
        },
        SpecShape::List(options) => FormShape::List {
            options: options.iter().map(|text| json!(text)).collect(),
        },
        SpecShape::Choice(options) => {
            FormShape::Choice(options.iter().map(|text| json!(text)).collect())
        }
        SpecShape::Nested(fields) => FormShape::Nested(Box::new(model(&SpecModel {
            ui_friendly_name: None,
            fields: *fields,
        }))),
        SpecShape::RawObject => FormShape::RawObject,
    }
}
