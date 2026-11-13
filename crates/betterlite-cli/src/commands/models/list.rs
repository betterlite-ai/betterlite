use std::io::Write;

use betterlite_models::controls::json_value::JsonValue;
use serde::Deserialize;

use crate::commands::Client;
use crate::commands::tables::{OutputFormat, write_table};

use super::format::{format_iso_datetime_str, format_timestamp};
use super::response::{fetch_data, write_json};
use super::value::cell_text;
use super::{ListModels, ModelsError};

const LIST_PATH: &str = "/models";
const HEADERS: [&str; 4] = ["ID", "Object", "Created", "Owned By"];

#[derive(Debug, Deserialize)]
struct ListedModel {
    #[serde(default)]
    id: Option<JsonValue>,
    #[serde(default)]
    object: Option<JsonValue>,
    #[serde(default)]
    created: Option<JsonValue>,
    #[serde(default)]
    owned_by: Option<JsonValue>,
}

pub(super) fn list_models(
    options: &ListModels,
    client: &Client,
    stdout: &mut dyn Write,
) -> Result<(), ModelsError> {
    let data = fetch_data(client, LIST_PATH)?;
    if options.format == OutputFormat::Json {
        return write_json(LIST_PATH, data.get().as_bytes(), stdout);
    }
    let models: Vec<ListedModel> =
        serde_json::from_str(data.get()).map_err(|source| ModelsError::Response {
            path: LIST_PATH,
            source,
        })?;
    let rows = models
        .iter()
        .map(list_row)
        .collect::<Result<Vec<_>, ModelsError>>()?;
    write_table("Available Models", &HEADERS, rows, stdout);
    Ok(())
}

fn list_row(model: &ListedModel) -> Result<Vec<String>, ModelsError> {
    let created = match model.created.as_ref() {
        None | Some(JsonValue::Null) => String::new(),
        Some(JsonValue::Number(number)) => {
            let seconds = number
                .as_i64()
                .ok_or(ModelsError::FieldType { field: "created" })?;
            format_timestamp(Some(seconds), "created")?
        }
        Some(JsonValue::Text(text)) => format_iso_datetime_str(Some(text), "created")?,
        Some(_) => return Err(ModelsError::FieldType { field: "created" }),
    };
    Ok(vec![
        model.id.as_ref().map_or_else(String::new, cell_text),
        model
            .object
            .as_ref()
            .map_or_else(|| String::from("model"), cell_text),
        created,
        model.owned_by.as_ref().map_or_else(String::new, cell_text),
    ])
}
