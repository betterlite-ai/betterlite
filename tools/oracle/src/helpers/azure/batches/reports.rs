use betterlite_azure::azure_urls::EndpointSelection;
use betterlite_azure::batches::error::AzureBatchesError;
use betterlite_azure::endpoints::projection::{ClientInputs, ClientKind, ClientOutcome, V1Key};
use betterlite_openai::batches::error::SdkError;
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};

pub(crate) fn raised(error: &AzureBatchesError) -> Value {
    match error {
        AzureBatchesError::Sdk { source } => sdk_raised(source),
        AzureBatchesError::Invalid { .. } => json!({ "raised": { "class": "ValidationError" } }),
        AzureBatchesError::Endpoint { source } => json!({ "raised": {
            "class": "OpenAIError", "message": source.to_string(),
        } }),
        AzureBatchesError::ClientMissing
        | AzureBatchesError::NotAsync
        | AzureBatchesError::CancelNotAsync
        | AzureBatchesError::NotSync => json!({ "raised": {
            "class": "ValueError", "message": error.to_string(),
        } }),
    }
}

fn sdk_raised(source: &SdkError) -> Value {
    match source {
        SdkError::Status {
            status,
            error_type,
            param,
            code,
            message,
        } => json!({ "raised": {
            "class": "APIStatusError", "status": status, "type": error_type,
            "param": param, "code": code, "message": message,
        } }),
        other => json!({ "raised": { "class": "APIError", "message": other.to_string() } }),
    }
}

// The SDK turns azure_endpoint into <endpoint>/openai and keeps a deployment base_url, always ending in a slash
fn azure_url(endpoint: &EndpointSelection) -> Option<String> {
    let raw = endpoint
        .azure_endpoint
        .as_ref()
        .or(endpoint.base_url.as_ref())?;
    let suffix = if endpoint.azure_endpoint.is_some() {
        "/openai/"
    } else {
        "/"
    };
    Some(format!("{}{suffix}", raw.trim_end_matches('/')))
}

#[derive(Default)]
struct Summary {
    key: Option<String>,
    retries: Option<i64>,
    timeout: Option<f64>,
    url: Option<String>,
    version: Option<String>,
}

const fn class_name(kind: ClientKind) -> &'static str {
    match kind {
        ClientKind::AzureOpenAi => "AzureOpenAI",
        ClientKind::AsyncAzureOpenAi => "AsyncAzureOpenAI",
        ClientKind::OpenAi => "OpenAI",
        ClientKind::AsyncOpenAi => "AsyncOpenAI",
        ClientKind::Other => "Other",
    }
}

// The client the case can read off the built SDK: family, key, defaults and the api-version it would send
pub(crate) fn projection(outcome: &ClientOutcome<'_>) -> Value {
    let mut summary = Summary::default();
    match outcome.inputs.as_ref() {
        Some(ClientInputs::Azure(azure)) => {
            summary.key = azure
                .api_key
                .as_ref()
                .map(|key| key.expose_secret().to_owned());
            summary.retries = Some(azure.max_retries);
            summary.timeout = azure.timeout;
            summary.url = azure_url(&azure.endpoint);
            summary.version = Some(azure.api_version.clone());
        }
        Some(ClientInputs::OpenAiV1(v1)) => {
            summary.key = match v1.key.as_ref() {
                Some(V1Key::Static(secret)) => Some(secret.expose_secret().to_owned()),
                Some(V1Key::Provider { .. }) | None => None,
            };
            summary.retries = Some(v1.max_retries);
            summary.timeout = v1.timeout;
            summary.url = v1.base_url.clone();
        }
        None => {}
    }
    json!({
        "class": class_name(outcome.kind),
        "api_key": summary.key,
        "max_retries": summary.retries,
        "organization": Value::Null,
        "timeout_default": summary.timeout.is_none(),
        "base_url": summary.url,
        "api_version": summary.version,
    })
}
