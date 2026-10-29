use betterlite_models::entry::ModelFields;
use betterlite_models::entry::generalizations::FallbackGeneralizations;
use serde::Deserialize;
use serde_json::Value;
use serde_json::value::RawValue;

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    source: String,
    #[serde(default)]
    kind: Kind,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    #[default]
    Entry,
    FallbackGeneralizations,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.kind {
            Kind::Entry => run_entry(&input.source),
            Kind::FallbackGeneralizations => run_fallback_generalizations(&input.source),
        }
    }
}

fn run_entry(source: &str) -> crate::helpers::HelperResult {
    let raw: Box<RawValue> =
        serde_json::from_str(source).map_err(|error| model_field_error(error.to_string()))?;
    let (fields, mut unknown_fields) =
        ModelFields::parse(raw.as_ref()).map_err(|error| model_field_error(error.to_string()))?;
    unknown_fields.sort();
    let mut projected =
        serde_json::to_value(&fields).map_err(|error| model_field_error(error.to_string()))?;
    if let Value::Object(object) = &mut projected {
        object.insert("unknown_fields".to_owned(), Value::from(unknown_fields));
    }
    Ok(projected)
}

fn run_fallback_generalizations(source: &str) -> crate::helpers::HelperResult {
    let generalizations: FallbackGeneralizations =
        serde_json::from_str(source).map_err(|error| model_field_error(error.to_string()))?;
    serde_json::to_value(&generalizations).map_err(|error| model_field_error(error.to_string()))
}

fn model_field_error(message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "ModelFieldError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

mod tests;
