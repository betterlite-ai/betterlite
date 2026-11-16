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

pub(crate) fn projection(outcome: &ClientOutcome<'_>) -> Value {
    let (api_key, max_retries, timeout) = match outcome.inputs.as_ref() {
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
    };
    json!({
        "class": class_name(outcome.kind),
        "api_key": api_key,
        "max_retries": max_retries,
        "organization": Value::Null,
        "timeout_default": timeout.is_none(),
    })
}
