use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum UrlError {
    #[error(
        "Missing Azure API Base - Please set `api_base` or `AZURE_API_BASE` environment variable. Expected format: https://<resource-name>.services.ai.azure.com/anthropic"
    )]
    MissingApiBase,
}
