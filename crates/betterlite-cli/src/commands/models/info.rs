use std::io::Write;

use betterlite_models::controls::json_value::JsonValue;
use serde::Deserialize;

use crate::commands::Client;
use crate::commands::tables::{OutputFormat, write_table};

use super::format::{format_cost_per_1k_tokens, format_iso_datetime_str};
use super::response::{fetch_data, write_json};
use super::value::{cell_text, cost_value, object_value, text_value};
use super::{GetModelsInfo, ModelsError};

const INFO_PATH: &str = "/v1/model/info";
const TITLE: &str = "Models Information";

#[derive(Clone, Copy)]
enum InfoColumn {
    PublicModel,
    UpstreamModel,
    CredentialName,
    CreatedAt,
    UpdatedAt,
    Id,
    InputCost,
    OutputCost,
}

impl InfoColumn {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "public_model" => Some(Self::PublicModel),
            "upstream_model" => Some(Self::UpstreamModel),
            "credential_name" => Some(Self::CredentialName),
            "created_at" => Some(Self::CreatedAt),
            "updated_at" => Some(Self::UpdatedAt),
            "id" => Some(Self::Id),
            "input_cost" => Some(Self::InputCost),
            "output_cost" => Some(Self::OutputCost),
            _ => None,
        }
    }

    const fn header(self) -> &'static str {
        match self {
            Self::PublicModel => "Public Model",
            Self::UpstreamModel => "Upstream Model",
            Self::CredentialName => "Credential Name",
            Self::CreatedAt => "Created At",
            Self::UpdatedAt => "Updated At",
            Self::Id => "ID",
            Self::InputCost => "Input Cost",
            Self::OutputCost => "Output Cost",
        }
    }
}

#[derive(Debug, Deserialize)]
struct InfoRow {
    #[serde(default)]
    model_name: Option<JsonValue>,
    #[serde(default)]
    litellm_params: Option<JsonValue>,
    #[serde(default)]
    model_info: Option<JsonValue>,
}

pub(super) fn get_models_info(
    options: &GetModelsInfo,
    client: &Client,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), ModelsError> {
    let data = fetch_data(client, INFO_PATH)?;
    if options.format == OutputFormat::Json {
        return write_json(INFO_PATH, data.get().as_bytes(), stdout);
    }
    let mut columns = Vec::new();
    for name in options.columns.split(',') {
        let name = name.trim();
        match InfoColumn::from_name(name) {
            Some(column) => columns.push(column),
            None => {
                let _warned = writeln!(stderr, "Warning: Unknown column '{name}'");
            }
        }
    }
    let models: Vec<InfoRow> =
        serde_json::from_str(data.get()).map_err(|source| ModelsError::Response {
            path: INFO_PATH,
            source,
        })?;
    let headers: Vec<&str> = columns.iter().map(|column| column.header()).collect();
    let rows = info_rows(&columns, &models)?;
    write_table(TITLE, &headers, rows, stdout);
    Ok(())
}

fn info_rows(columns: &[InfoColumn], models: &[InfoRow]) -> Result<Vec<Vec<String>>, ModelsError> {
    if columns.is_empty() {
        return Ok(Vec::new());
    }
    let mut rows = Vec::with_capacity(models.len());
    for model in models {
        let mut row = Vec::with_capacity(columns.len());
        for column in columns {
            row.push(info_cell(*column, model)?);
        }
        rows.push(row);
    }
    Ok(rows)
}

fn info_cell(column: InfoColumn, model: &InfoRow) -> Result<String, ModelsError> {
    match column {
        InfoColumn::PublicModel => Ok(model
            .model_name
            .as_ref()
            .map_or_else(String::new, cell_text)),
        InfoColumn::UpstreamModel => {
            let params = object_value(model.litellm_params.as_ref(), "litellm_params")?;
            Ok(params
                .and_then(|params| params.get("model"))
                .map_or_else(String::new, cell_text))
        }
        InfoColumn::CredentialName => {
            let params = object_value(model.litellm_params.as_ref(), "litellm_params")?;
            Ok(params
                .and_then(|params| params.get("litellm_credential_name"))
                .map_or_else(String::new, cell_text))
        }
        InfoColumn::CreatedAt => {
            let info = object_value(model.model_info.as_ref(), "model_info")?;
            let text = text_value(info.and_then(|info| info.get("created_at")), "created_at")?;
            format_iso_datetime_str(text, "created_at")
        }
        InfoColumn::UpdatedAt => {
            let info = object_value(model.model_info.as_ref(), "model_info")?;
            let text = text_value(info.and_then(|info| info.get("updated_at")), "updated_at")?;
            format_iso_datetime_str(text, "updated_at")
        }
        InfoColumn::Id => {
            let info = object_value(model.model_info.as_ref(), "model_info")?;
            Ok(info
                .and_then(|info| info.get("id"))
                .map_or_else(String::new, cell_text))
        }
        InfoColumn::InputCost => {
            let info = object_value(model.model_info.as_ref(), "model_info")?;
            let cost = cost_value(
                info.and_then(|info| info.get("input_cost_per_token")),
                "input_cost_per_token",
            )?;
            format_cost_per_1k_tokens(cost, "input_cost_per_token")
        }
        InfoColumn::OutputCost => {
            let info = object_value(model.model_info.as_ref(), "model_info")?;
            let cost = cost_value(
                info.and_then(|info| info.get("output_cost_per_token")),
                "output_cost_per_token",
            )?;
            format_cost_per_1k_tokens(cost, "output_cost_per_token")
        }
    }
}
