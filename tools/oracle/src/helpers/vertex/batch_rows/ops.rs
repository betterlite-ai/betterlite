use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_vertex::vertex_batch_rows::{
    decode::{
        decode_vertex_batch_row, is_embeddings_output_row, is_generate_output_row,
        split_vertex_batch_key,
    },
    embeddings::regroup_embeddings,
    labels::{custom_id_from_labels, set_custom_id_labels},
    paths::{is_passthrough_managed_gcs_url, model_from_managed_gcs_url},
    types::VertexBatchRow,
};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::helpers::OrHelperError as _;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Encode {
        custom_ids: Vec<String>,
    },
    Rows {
        rows: Vec<Json>,
    },
    Regroup {
        rows: Vec<Json>,
        model: Option<String>,
    },
    Urls {
        urls: Vec<String>,
    },
}

pub(crate) fn run(input: &Input) -> crate::helpers::HelperResult {
    match input {
        Input::Encode { custom_ids } => encode(custom_ids),
        Input::Rows { rows } => batch_rows(rows),
        Input::Regroup { rows, model } => regroup(rows, model.as_deref()),
        Input::Urls { urls } => Ok(json!({"rows": urls.iter().map(|url| json!({
            "passthrough": is_passthrough_managed_gcs_url(url),
            "model": model_from_managed_gcs_url(url),
        })).collect::<Vec<_>>()})),
    }
}

fn encode(custom_ids: &[String]) -> crate::helpers::HelperResult {
    let mut rows = Vec::with_capacity(custom_ids.len());
    for custom_id in custom_ids {
        let mut labels: IndexMap<String, Json> = IndexMap::new();
        set_custom_id_labels(&mut labels, custom_id);
        let resolved = custom_id_from_labels(&labels).or_value_error()?;
        rows.push(json!({"labels": labels, "custom_id": resolved}));
    }
    Ok(json!({"rows": rows}))
}

fn decode_rows(values: &[Json]) -> Result<Vec<VertexBatchRow>, Box<HelperError>> {
    let mut rows = Vec::with_capacity(values.len());
    for value in values {
        let bytes = serde_json::to_vec(value).or_value_error()?;
        rows.push(decode_vertex_batch_row(&bytes).or_value_error()?);
    }
    Ok(rows)
}

fn batch_rows(values: &[Json]) -> crate::helpers::HelperResult {
    let mut found = Vec::with_capacity(values.len());
    for row in decode_rows(values)? {
        let key = split_vertex_batch_key(&row).or_value_error()?;
        found.push(json!({
            "custom_id": key.custom_id,
            "embeddings": is_embeddings_output_row(&row),
            "generate": is_generate_output_row(&row),
            "index": key.index,
            "total": key.total,
        }));
    }
    Ok(json!({"rows": found}))
}

fn regroup(values: &[Json], model: Option<&str>) -> crate::helpers::HelperResult {
    let mut ids = (1..).map(Uuid::from_u128);
    let rows = decode_rows(values)?;
    let outputs = regroup_embeddings(&rows, model, &mut || ids.next().unwrap_or_default())
        .or_value_error()?;
    Ok(json!({"rows": outputs}))
}
