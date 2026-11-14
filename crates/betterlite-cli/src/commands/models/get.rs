use std::io::Write;

use betterlite_models::controls::json_value::JsonValue;
use serde::Deserialize;
use serde_json::value::RawValue;

use crate::commands::Client;
use crate::commands::tables::write_json_response;

use super::response::fetch_data;
use super::{GetModel, ModelsError};

const INFO_PATH: &str = "/v1/model/info";

#[derive(Debug, Deserialize)]
struct Candidate {
    #[serde(default)]
    model_name: Option<JsonValue>,
    #[serde(default)]
    model_info: Option<JsonValue>,
}

pub(super) fn get_model(
    options: &GetModel,
    client: &Client,
    stdout: &mut dyn Write,
) -> Result<(), ModelsError> {
    if options.model_id.is_none() && options.model_name.is_none() {
        return Err(ModelsError::MissingSelector);
    }
    if options.model_id.is_some() && options.model_name.is_some() {
        return Err(ModelsError::BothSelectors);
    }
    let data = fetch_data(client, INFO_PATH)?;
    let models: Vec<Box<RawValue>> =
        serde_json::from_str(data.get()).map_err(|source| ModelsError::Response {
            path: INFO_PATH,
            source,
        })?;
    for model in models {
        let candidate: Candidate =
            serde_json::from_str(model.get()).map_err(|source| ModelsError::Response {
                path: INFO_PATH,
                source,
            })?;
        if matches_selector(options, &candidate) {
            return write_json_response(model.get().as_bytes(), stdout).map_err(|source| {
                ModelsError::Response {
                    path: INFO_PATH,
                    source,
                }
            });
        }
    }
    Err(ModelsError::NotFound {
        selector: selector(options),
    })
}

fn matches_selector(options: &GetModel, candidate: &Candidate) -> bool {
    match (&options.model_id, &options.model_name) {
        (Some(model_id), None) => id_matches(candidate.model_info.as_ref(), model_id),
        (None, Some(model_name)) => text_matches(candidate.model_name.as_ref(), model_name),
        (Some(_), Some(_)) | (None, None) => false,
    }
}

fn id_matches(info: Option<&JsonValue>, model_id: &str) -> bool {
    let Some(JsonValue::Object(fields)) = info else {
        return false;
    };
    text_matches(fields.get("id"), model_id)
}

fn text_matches(value: Option<&JsonValue>, expected: &str) -> bool {
    let Some(JsonValue::Text(text)) = value else {
        return false;
    };
    text == expected
}

fn selector(options: &GetModel) -> String {
    match (&options.model_id, &options.model_name) {
        (Some(model_id), _) => format!("Model with id={model_id}"),
        (None, Some(model_name)) => format!("Model with model_name={model_name}"),
        (None, None) => String::from("Unknown error trying to find model"),
    }
}
