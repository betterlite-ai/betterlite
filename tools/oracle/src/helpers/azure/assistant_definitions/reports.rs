use betterlite_azure::azure_assistant_definitions::client::AzureAssistantsClient;
use betterlite_azure::azure_assistant_definitions::error::DefinitionsError;
use betterlite_openai::assistants::client::ClientKind;
use betterlite_openai::assistants::error::AssistantsError;
use serde::Serialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::fixture::Wire;

pub(super) fn invalid_input(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

// What the Python helper reports for the errors the Azure SDK raises.
pub(super) fn failure(error: &DefinitionsError) -> Result<Value, Box<HelperError>> {
    match error {
        DefinitionsError::Assistants(AssistantsError::Status { status, .. }) => {
            Ok(json!({"kind": "status", "status": status}))
        }
        DefinitionsError::MissingCredentials => Ok(json!({"kind": "configuration"})),
        DefinitionsError::Assistants(
            AssistantsError::EmptyAssistantId | AssistantsError::DotSegment { .. },
        )
        | DefinitionsError::MissingEndpoint
        | DefinitionsError::ProviderReply
        | DefinitionsError::Timeout { .. } => Ok(json!({"kind": "value"})),
        other => Err(invalid_input(other.to_string())),
    }
}

const fn kind_name(kind: ClientKind) -> &'static str {
    match kind {
        ClientKind::Sync => "sync",
        ClientKind::Async => "async",
    }
}

pub(super) fn operation<T: Serialize>(
    wire: &Wire,
    outcome: Result<(ClientKind, T), DefinitionsError>,
) -> HelperResult {
    let requests = wire.take();
    match outcome {
        Ok((kind, reply)) => Ok(json!({
            "requests": requests,
            "kind": kind_name(kind),
            "result": serde_json::to_value(reply).unwrap_or(Value::Null),
        })),
        Err(error) => Ok(json!({"requests": requests, "error": failure(&error)?})),
    }
}

pub(super) fn client_report<T>(client: &AzureAssistantsClient<'_, T>) -> Value {
    let source = if client.inputs.is_some() {
        "constructed"
    } else {
        "supplied"
    };
    let max_retries = client
        .inputs
        .as_ref()
        .map_or(Value::Null, |inputs| json!(inputs.max_retries));
    json!({
        "kind": kind_name(client.kind),
        "source": source,
        "base_url": client.base_url.trim_end_matches('/'),
        "max_retries": max_retries,
        "timeout": client.deadline.as_secs_f64(),
    })
}

pub(super) fn client_failure(kind: ClientKind, error: &DefinitionsError) -> HelperResult {
    Ok(json!({
        "kind": kind_name(kind),
        "source": "constructed",
        "error": failure(error)?,
    }))
}
