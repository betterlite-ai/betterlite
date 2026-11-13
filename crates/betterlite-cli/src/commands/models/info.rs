use std::io::Write;

use betterlite_models::controls::json_value::JsonValue;
use serde::Deserialize;

use crate::commands::Client;
use crate::commands::tables::{OutputFormat, write_json_response, write_table};

use super::format::{format_cost_per_1k_tokens, format_iso_datetime_str};
use super::response::fetch_data;
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

struct ColumnSpec {
    header: &'static str,
    field: &'static str,
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

    const fn spec(self) -> ColumnSpec {
        match self {
            Self::PublicModel => ColumnSpec {
                header: "Public Model",
                field: "model_name",
            },
            Self::UpstreamModel => ColumnSpec {
                header: "Upstream Model",
                field: "model",
            },
            Self::CredentialName => ColumnSpec {
                header: "Credential Name",
                field: "litellm_credential_name",
            },
            Self::CreatedAt => ColumnSpec {
                header: "Created At",
                field: "created_at",
            },
            Self::UpdatedAt => ColumnSpec {
                header: "Updated At",
                field: "updated_at",
            },
            Self::Id => ColumnSpec {
                header: "ID",
                field: "id",
            },
            Self::InputCost => ColumnSpec {
                header: "Input Cost",
                field: "input_cost_per_token",
            },
            Self::OutputCost => ColumnSpec {
                header: "Output Cost",
                field: "output_cost_per_token",
            },
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
        return write_json_response(data.get().as_bytes(), stdout).map_err(|source| {
            ModelsError::Response {
                path: INFO_PATH,
                source,
            }
        });
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
    let headers: Vec<&str> = columns.iter().map(|column| column.spec().header).collect();
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
    let field = column.spec().field;
    match column {
        InfoColumn::PublicModel => Ok(model
            .model_name
            .as_ref()
            .map_or_else(String::new, cell_text)),
        InfoColumn::UpstreamModel | InfoColumn::CredentialName => {
            let params = object_value(model.litellm_params.as_ref(), "litellm_params")?;
            Ok(params
                .and_then(|params| params.get(field))
                .map_or_else(String::new, cell_text))
        }
        InfoColumn::Id => {
            let info = object_value(model.model_info.as_ref(), "model_info")?;
            Ok(info
                .and_then(|info| info.get(field))
                .map_or_else(String::new, cell_text))
        }
        InfoColumn::CreatedAt | InfoColumn::UpdatedAt => {
            let info = object_value(model.model_info.as_ref(), "model_info")?;
            let text = text_value(info.and_then(|info| info.get(field)), field)?;
            format_iso_datetime_str(text, field)
        }
        InfoColumn::InputCost | InfoColumn::OutputCost => {
            let info = object_value(model.model_info.as_ref(), "model_info")?;
            let cost = cost_value(info.and_then(|info| info.get(field)), field)?;
            format_cost_per_1k_tokens(cost, field)
        }
    }
}
