use std::collections::BTreeMap;

use comfy_table::Table;
use http::Method;
use serde::Deserialize;
use serde_json::value::RawValue;

use crate::commands::{Client, Payload, Request};

use super::{ChatError, Terminal};

const MODELS_PATH: &str = "/models";
const MAX_DISPLAYED_MODELS: usize = 200;
const NAME_PROMPT: &str = "Please enter a model name: ";
const INDEX_PROMPT: &str =
    "Select a model by entering the index number (or type a model name directly): ";

#[derive(Deserialize)]
struct ModelRow {
    #[serde(default)]
    id: String,
    #[serde(default)]
    owned_by: String,
}

pub(super) fn pick(
    client: &Client,
    terminal: &mut dyn Terminal,
) -> Result<Option<String>, ChatError> {
    let models = match fetch_models(client) {
        Ok(models) => models,
        Err(error) => {
            terminal.write_error_line(&format!("Warning: Could not fetch models list: {error}"))?;
            Vec::new()
        }
    };
    if models.is_empty() {
        terminal.write_line("No models available or could not fetch models list.")?;
        terminal.write_fragment(NAME_PROMPT)?;
        return match terminal.read_line()? {
            Some(line) if !line.trim().is_empty() => Ok(Some(line)),
            _ => Ok(None),
        };
    }
    write_models_table(terminal, &models)?;
    select_by_prompt(terminal, &models)
}

fn fetch_models(client: &Client) -> Result<Vec<ModelRow>, ChatError> {
    let request = Request {
        method: Method::GET,
        url: client.resolve(MODELS_PATH)?,
        headers: Vec::new(),
        payload: Payload::Empty,
    };
    let response = client.send(request)?;
    if response.status >= 400 {
        return Err(ChatError::Status {
            status: response.status,
        });
    }
    parse_models(&response.body)
}

fn parse_models(body: &[u8]) -> Result<Vec<ModelRow>, ChatError> {
    let fields: BTreeMap<String, Box<RawValue>> =
        serde_json::from_slice(body).map_err(ChatError::Models)?;
    let Some(data) = fields.get("data") else {
        return Err(ChatError::ModelsData);
    };
    let Ok(items) = serde_json::from_str::<Vec<&RawValue>>(data.get()) else {
        return Ok(Vec::new());
    };
    Ok(items
        .iter()
        .filter_map(|item| serde_json::from_str::<ModelRow>(item.get()).ok())
        .collect())
}

fn write_models_table(terminal: &mut dyn Terminal, models: &[ModelRow]) -> Result<(), ChatError> {
    if models.len() > MAX_DISPLAYED_MODELS {
        let hidden = models.len().saturating_sub(MAX_DISPLAYED_MODELS);
        terminal.write_line(&format!("... and {hidden} more models"))?;
    }
    let mut table = Table::new();
    table.set_header(vec!["Index", "Model ID", "Owned By"]);
    for (index, model) in (1_usize..).zip(models.iter().take(MAX_DISPLAYED_MODELS)) {
        table.add_row(vec![
            index.to_string(),
            model.id.clone(),
            model.owned_by.clone(),
        ]);
    }
    terminal.write_line("Available Models")?;
    for line in table.to_string().lines() {
        terminal.write_line(line)?;
    }
    Ok(())
}

fn select_by_prompt(
    terminal: &mut dyn Terminal,
    models: &[ModelRow],
) -> Result<Option<String>, ChatError> {
    loop {
        terminal.write_fragment(INDEX_PROMPT)?;
        let Some(line) = terminal.read_line()? else {
            terminal.write_line("Model selection cancelled.")?;
            return Ok(None);
        };
        if line.is_empty() {
            return Ok(models.first().map(|model| model.id.clone()));
        }
        let choice = line.trim();
        if choice.is_empty() {
            terminal.write_line("Please enter a valid model name or index")?;
            continue;
        }
        let Ok(number) = choice.parse::<i64>() else {
            return Ok(Some(choice.to_owned()));
        };
        let index = number
            .checked_sub(1)
            .and_then(|value| usize::try_from(value).ok());
        if let Some(picked) = index.and_then(|position| models.get(position)) {
            return Ok(Some(picked.id.clone()));
        }
        terminal.write_line(&format!(
            "Invalid index. Please enter a number between 1 and {}",
            models.len()
        ))?;
    }
}
