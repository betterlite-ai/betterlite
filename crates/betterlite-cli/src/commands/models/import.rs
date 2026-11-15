use std::fs;
use std::io::Write;

use betterlite_models::controls::json_value::JsonValue;
use comfy_table::{Attribute, Cell, Table};
use indexmap::IndexMap;
use serde::Serialize;

use crate::commands::Client;
use crate::commands::tables::write_table;

use super::response::post_json;
use super::value::ModelObject;
use super::{ImportModels, ModelsError};

mod filter;

use filter::{ACCESS_REGEX_FLAG, MODEL_REGEX_FLAG, compile, filter_model};

const NEW_PATH: &str = "/model/new";
const MODELS_TITLE: &str = "Models Imported";
const DRY_RUN_TITLE: &str = "Models that would be imported if --dry-run was not provided";

#[derive(Serialize)]
struct NewModelBody<'model> {
    model_name: &'model str,
    litellm_params: &'model ModelObject,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_info: Option<&'model ModelObject>,
}

struct ModelYamlInfo<'model> {
    model_name: &'model str,
    model_params: &'model ModelObject,
    model_info: Option<&'model ModelObject>,
    model_id: &'model str,
    access_groups: Vec<&'model str>,
    provider: &'model str,
}

pub(super) fn import_models(
    options: &ImportModels,
    client: &Client,
    stdout: &mut dyn Write,
) -> Result<(), ModelsError> {
    let document = read_document(options)?;
    let JsonValue::Object(root) = document else {
        return Err(ModelsError::MissingModelList);
    };
    let Some(model_list) = root.get("model_list") else {
        return Err(ModelsError::MissingModelList);
    };
    let JsonValue::Array(entries) = model_list else {
        return Err(ModelsError::ModelListNotList);
    };
    let mut added = Vec::new();
    let counts = import_entries(entries, options, client, &mut added)?;
    let title = if options.dry_run {
        DRY_RUN_TITLE
    } else {
        MODELS_TITLE
    };
    print_models_table(&added, title, stdout);
    print_summary_table(&counts, stdout)
}

fn read_document(options: &ImportModels) -> Result<JsonValue, ModelsError> {
    let text = fs::read_to_string(&options.yaml_file).map_err(|source| ModelsError::Read {
        path: options.yaml_file.clone(),
        source,
    })?;
    if text.trim().is_empty() {
        return Err(ModelsError::MissingModelList);
    }
    serde_saphyr::from_str(&text).map_err(|source| ModelsError::Yaml { source })
}

fn import_entries<'entries>(
    entries: &'entries [JsonValue],
    options: &ImportModels,
    client: &Client,
    added: &mut Vec<ModelYamlInfo<'entries>>,
) -> Result<IndexMap<String, usize>, ModelsError> {
    let model_regex = compile(options.model_regex.as_deref(), MODEL_REGEX_FLAG)?;
    let access_regex = compile(options.access_group_regex.as_deref(), ACCESS_REGEX_FLAG)?;
    for (index, entry) in entries.iter().enumerate() {
        let JsonValue::Object(model) = entry else {
            return Err(ModelsError::EntryType { index });
        };
        if filter_model(model, model_regex.as_ref(), access_regex.as_ref(), index)? {
            added.push(yaml_info(model, index)?);
        }
    }
    let mut counts: IndexMap<String, usize> = IndexMap::new();
    for info in added.iter() {
        if !options.dry_run {
            send_new_model(client, info)?;
        }
        let count = counts.entry(info.provider.to_owned()).or_insert(0);
        *count = count.checked_add(1).ok_or(ModelsError::Count)?;
    }
    Ok(counts)
}

fn send_new_model(client: &Client, info: &ModelYamlInfo<'_>) -> Result<(), ModelsError> {
    let body = serde_json::to_vec(&NewModelBody {
        model_name: info.model_name,
        litellm_params: info.model_params,
        model_info: info.model_info.filter(|model_info| !model_info.is_empty()),
    })
    .map_err(ModelsError::Encode)?;
    let _ignored = post_json(client, NEW_PATH, body);
    Ok(())
}

fn yaml_info(model: &ModelObject, index: usize) -> Result<ModelYamlInfo<'_>, ModelsError> {
    let Some(JsonValue::Text(model_name)) = model.get("model_name") else {
        return Err(ModelsError::EntryField {
            index,
            field: "model_name",
        });
    };
    let model_params =
        field_object(model, "litellm_params", index)?.ok_or(ModelsError::EntryField {
            index,
            field: "litellm_params",
        })?;
    let Some(JsonValue::Text(model_id)) = model_params.get("model") else {
        return Err(ModelsError::EntryField {
            index,
            field: "model",
        });
    };
    let model_info = field_object(model, "model_info", index)?;
    let access_groups = access_group_names(model_info, index)?;
    let provider = model_id
        .split_once('/')
        .map_or(model_id.as_str(), |(before, _rest)| before);
    Ok(ModelYamlInfo {
        model_name,
        model_params,
        model_info,
        model_id,
        access_groups,
        provider,
    })
}

fn field_object<'model>(
    object: &'model ModelObject,
    field: &'static str,
    index: usize,
) -> Result<Option<&'model ModelObject>, ModelsError> {
    match object.get(field) {
        Some(JsonValue::Object(fields)) => Ok(Some(fields)),
        None | Some(JsonValue::Null) => Ok(None),
        Some(_) => Err(ModelsError::EntryField { index, field }),
    }
}

fn access_group_names(
    model_info: Option<&ModelObject>,
    index: usize,
) -> Result<Vec<&str>, ModelsError> {
    let groups = match model_info.and_then(|info| info.get("access_groups")) {
        None | Some(JsonValue::Null) => return Ok(Vec::new()),
        Some(JsonValue::Array(groups)) => groups,
        Some(_) => {
            return Err(ModelsError::EntryField {
                index,
                field: "access_groups",
            });
        }
    };
    groups
        .iter()
        .map(|group| match group {
            JsonValue::Text(text) => Ok(text.as_str()),
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_) => Err(ModelsError::EntryField {
                index,
                field: "access_groups",
            }),
        })
        .collect()
}

fn print_models_table(models: &[ModelYamlInfo<'_>], title: &str, stdout: &mut dyn Write) {
    if models.is_empty() {
        return;
    }
    let rows: Vec<Vec<String>> = models
        .iter()
        .map(|model| {
            vec![
                model.model_name.to_owned(),
                model.model_id.to_owned(),
                model.access_groups.join(", "),
            ]
        })
        .collect();
    write_table(
        title,
        &["Model Name", "Upstream Model", "Access Groups"],
        rows,
        stdout,
    );
}

fn summary_table(counts: &IndexMap<String, usize>) -> Result<Table, ModelsError> {
    let mut table = Table::new();
    table.set_header(vec!["Provider", "Count"]);
    for (provider, count) in counts {
        table.add_row(vec![Cell::new(provider), Cell::new(count)]);
    }
    let mut total = 0_usize;
    for count in counts.values() {
        total = total.checked_add(*count).ok_or(ModelsError::Count)?;
    }
    table.add_row(vec![
        Cell::new("Total").add_attribute(Attribute::Bold),
        Cell::new(total).add_attribute(Attribute::Bold),
    ]);
    Ok(table)
}

fn print_summary_table(
    counts: &IndexMap<String, usize>,
    stdout: &mut dyn Write,
) -> Result<(), ModelsError> {
    let table = summary_table(counts)?;
    let _wrote_title = writeln!(stdout, "Model Import Summary");
    let _wrote_table = writeln!(stdout, "{table}");
    Ok(())
}

mod tests;
