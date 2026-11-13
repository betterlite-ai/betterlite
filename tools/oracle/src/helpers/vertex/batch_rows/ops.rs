use betterlite_models::decisions::Json;
use betterlite_vertex::vertex_batch_rows::correlation::custom_id_from_row;
use betterlite_vertex::vertex_batch_rows::decode::{
    decode_vertex_batch_row, is_embeddings_output_row, is_generate_output_row,
    split_vertex_batch_key,
};
use betterlite_vertex::vertex_batch_rows::embeddings::regroup_embeddings;
use betterlite_vertex::vertex_batch_rows::labels::{custom_id_from_labels, set_custom_id_labels};
use betterlite_vertex::vertex_batch_rows::paths::{
    is_passthrough_managed_gcs_url, model_from_managed_gcs_url,
};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::helpers::OrHelperError as _;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Encode {
        custom_ids: Vec<String>,
    },
    Resolve {
        rows: Vec<Json>,
    },
    Split {
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
        Input::Resolve { rows } => resolve(rows),
        Input::Split { rows } => split(rows),
        Input::Regroup { rows, model } => regroup(rows, model.as_deref()),
        Input::Urls { urls } => Ok(managed_urls(urls)),
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

fn resolve(rows: &[Json]) -> crate::helpers::HelperResult {
    let mut found = Vec::with_capacity(rows.len());
    for value in rows {
        let bytes = serde_json::to_vec(value).or_value_error()?;
        let row = decode_vertex_batch_row(&bytes).or_value_error()?;
        let custom_id = custom_id_from_row(&row).or_value_error()?;
        found.push(json!({
            "custom_id": custom_id,
            "embeddings": is_embeddings_output_row(&row),
            "generate": is_generate_output_row(&row),
        }));
    }
    Ok(json!({"rows": found}))
}

fn split(rows: &[Json]) -> crate::helpers::HelperResult {
    let mut found = Vec::with_capacity(rows.len());
    for value in rows {
        let bytes = serde_json::to_vec(value).or_value_error()?;
        let row = decode_vertex_batch_row(&bytes).or_value_error()?;
        let key = split_vertex_batch_key(&row).or_value_error()?;
        found.push(json!([key.custom_id, key.index, key.total]));
    }
    Ok(json!({"rows": found}))
}

fn regroup(rows: &[Json], model: Option<&str>) -> crate::helpers::HelperResult {
    let mut decoded = Vec::with_capacity(rows.len());
    for value in rows {
        let bytes = serde_json::to_vec(value).or_value_error()?;
        decoded.push(decode_vertex_batch_row(&bytes).or_value_error()?);
    }
    let mut next = 0_u128;
    let outputs = regroup_embeddings(&decoded, model, || {
        next = next.saturating_add(1);
        Uuid::from_u128(next)
    })
    .or_value_error()?;
    Ok(json!({"rows": outputs}))
}

fn managed_urls(urls: &[String]) -> Value {
    let mut rows = Vec::with_capacity(urls.len());
    for url in urls {
        rows.push(json!({
            "passthrough": is_passthrough_managed_gcs_url(url),
            "model": model_from_managed_gcs_url(url),
        }));
    }
    json!({"rows": rows})
}
