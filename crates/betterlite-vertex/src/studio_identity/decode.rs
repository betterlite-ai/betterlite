use serde::Deserialize;

use super::error::StudioIdentityError;
use super::process_model_names;

pub const MODEL_LIST_LIMIT: usize = 4 * 1024 * 1024;

#[derive(Deserialize)]
struct ModelList {
    models: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    name: String,
}

pub fn decode_studio_models(status: u16, bytes: &[u8]) -> Result<Vec<String>, StudioIdentityError> {
    if bytes.len() > MODEL_LIST_LIMIT {
        return Err(StudioIdentityError::ResponseTooLarge {
            limit: MODEL_LIST_LIMIT,
        });
    }
    if status != 200 {
        let body = str::from_utf8(bytes).map_or_else(
            |_| format!("{} bytes that are not UTF-8", bytes.len()),
            str::to_owned,
        );
        return Err(StudioIdentityError::Status { status, body });
    }
    let list: ModelList = serde_json::from_slice(bytes)?;
    let names = list
        .models
        .into_iter()
        .map(|model| model.name)
        .collect::<Vec<_>>();
    Ok(process_model_names(&names))
}
