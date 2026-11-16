use betterlite_azure::endpoints::projection::{ClientInputs, ClientKind, ClientOutcome, V1Key};
use betterlite_azure::fine_tuning::error::AzureFineTuneError;
use secrecy::ExposeSecret;
use serde_json::{Value, json};

pub(crate) fn raised(error: &AzureFineTuneError) -> Value {
    match error {
        AzureFineTuneError::Sdk { source } => json!({ "raised": {
            "class": "APIStatusError", "status": source.status, "type": source.error_type,
            "param": source.param, "code": source.code, "message": source.message,
        } }),
        AzureFineTuneError::Build { .. } => json!({ "raised": { "class": "TypeError" } }),
        AzureFineTuneError::Decode { .. } => json!({ "raised": { "class": "ValidationError" } }),
        AzureFineTuneError::Endpoint { .. }
        | AzureFineTuneError::ClientMissing
        | AzureFineTuneError::NotAsyncClient => {
            json!({ "raised": { "class": "ValueError", "message": error.to_string() } })
        }
    }
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

fn v1_key(key: Option<&V1Key<'_>>) -> Option<String> {
    match key {
        Some(V1Key::Static(secret)) => Some(secret.expose_secret().to_owned()),
        Some(V1Key::Provider { .. }) | None => None,
    }
}

fn client_inputs(outcome: &ClientOutcome<'_>) -> (Option<String>, Option<i64>, Option<f64>) {
    match outcome.inputs.as_ref() {
        Some(ClientInputs::Azure(azure)) => (
            azure
                .api_key
                .as_ref()
                .map(|key| key.expose_secret().to_owned()),
            Some(azure.max_retries),
            azure.timeout,
        ),
        Some(ClientInputs::OpenAiV1(v1)) => {
            (v1_key(v1.key.as_ref()), Some(v1.max_retries), v1.timeout)
        }
        None => (None, None, None),
    }
}

// the SDK turns azure_endpoint into <endpoint>/openai and keeps a deployment base_url, always ending in a slash
fn rebuilt(inputs: Option<&ClientInputs<'_>>) -> (Option<String>, Option<String>) {
    let Some(inputs) = inputs else {
        return (None, None);
    };
    match inputs {
        ClientInputs::Azure(azure) => {
            let base_url = match (&azure.endpoint.azure_endpoint, &azure.endpoint.base_url) {
                (Some(endpoint), _) => Some(format!("{}/openai/", endpoint.trim_end_matches('/'))),
                (None, Some(path)) => Some(format!("{}/", path.trim_end_matches('/'))),
                (None, None) => None,
            };
            (base_url, Some(azure.api_version.clone()))
        }
        ClientInputs::OpenAiV1(v1) => (v1.base_url.clone(), None),
    }
}

pub(crate) fn projection(outcome: &ClientOutcome<'_>) -> Value {
    let (api_key, max_retries, timeout) = client_inputs(outcome);
    let (base_url, api_version) = rebuilt(outcome.inputs.as_ref());
    json!({
        "class": class_name(outcome.kind),
        "api_key": api_key,
        "max_retries": max_retries,
        "organization": Value::Null,
        "timeout_default": timeout.is_none(),
        "base_url": base_url,
        "api_version": api_version,
    })
}
