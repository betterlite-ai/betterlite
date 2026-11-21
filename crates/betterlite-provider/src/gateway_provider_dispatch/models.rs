use std::fmt::Display;

use betterlite_openai::vercel_ai_gateway::models::{models_url, parse_models};
use betterlite_openai::vercel_ai_gateway::models_error::ModelsError as VercelModelsError;
use thiserror::Error;

use super::context::DispatchSettings;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelsRequest {
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelsResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

// the HTTP client the listing runs on, injected so a test can record the request
pub trait ModelsTransport: Send + Sync {
    type Error: Display;

    fn get(&self, request: &ModelsRequest) -> Result<ModelsResponse, Self::Error>;
}

#[derive(Debug, Error)]
pub enum ModelsError {
    #[error(transparent)]
    Reply(#[from] VercelModelsError),
    #[error("the models request failed: {message}")]
    Request { message: String },
}

// VercelAIGatewayConfig.get_models: a GET to the passed base, else VERCEL_AI_GATEWAY_API_BASE, else the public default, with no Authorization header
pub fn get_models<T>(
    transport: &T,
    api_base: Option<&str>,
    settings: DispatchSettings<'_>,
) -> Result<Vec<String>, ModelsError>
where
    T: ModelsTransport,
{
    let environment = settings.vercel_environment();
    let request = ModelsRequest {
        url: models_url(api_base, &environment),
    };
    let response = transport
        .get(&request)
        .map_err(|error| ModelsError::Request {
            message: error.to_string(),
        })?;
    Ok(parse_models(response.status, &response.body)?)
}
