use std::str;

use serde::Deserialize;

use crate::vercel_ai_gateway::VercelAiGatewayEnvironment;
use crate::vercel_ai_gateway::models_error::ModelsError;
use crate::vercel_ai_gateway::provider_info::resolve_api_base;

#[derive(Deserialize)]
struct ModelList {
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
}

#[must_use]
pub fn models_url(api_base: Option<&str>, environment: &VercelAiGatewayEnvironment) -> String {
    let mut url = resolve_api_base(api_base, environment);
    url.push_str("/models");
    url
}

pub fn parse_models(status: u16, body: &[u8]) -> Result<Vec<String>, ModelsError> {
    if status != 200 {
        let text = str::from_utf8(body).map_or_else(
            |_| format!("{} bytes that are not UTF-8", body.len()),
            str::to_owned,
        );
        return Err(ModelsError::Status { code: status, text });
    }
    let list: ModelList = serde_json::from_slice(body).map_err(ModelsError::Shape)?;
    Ok(list.data.into_iter().map(|entry| entry.id).collect())
}
