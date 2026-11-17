use betterlite_logging::python_json::dumps_object;
use betterlite_models::decisions::Json;
use secrecy::{ExposeSecret as _, SecretString};

mod tests;

// the deployment litellm_params the read resolver reads; LiteLLM checks their type at run time, so they stay JSON
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReadDeployment {
    pub gcs_bucket_name: Option<Json>,
    pub bucket_name: Option<Json>,
    pub vertex_credentials: Option<Json>,
}

// the environment values the read resolver reads, where an empty variable counts as unset
#[derive(Debug, Clone, Default)]
pub struct ReadEnvironment {
    pub batch_bucket_name: Option<String>,
    pub bucket_name: Option<String>,
    pub path_service_account: Option<SecretString>,
}

#[derive(Debug, Clone, Default)]
pub struct ReadSettings {
    pub bucket_name: Option<String>,
    pub service_account: Option<SecretString>,
}

// LiteLLM's `or` over the untyped litellm_params skips every false JSON value, a 0, "" or empty list included
fn has_value(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number.as_f64().is_some_and(|amount| amount.abs() > 0.0),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}

fn environment_text(value: Option<&String>) -> Option<String> {
    let text = value?;
    if text.is_empty() {
        return None;
    }
    Some(text.clone())
}

// _resolve_read_gcs_config: a nonstring deployment bucket or credential falls to the environment, never to the alias
#[must_use]
pub fn vertex_file_read_settings(
    deployment: &ReadDeployment,
    argument: Option<&Json>,
    environment: &ReadEnvironment,
) -> ReadSettings {
    let bucket = deployment
        .gcs_bucket_name
        .as_ref()
        .filter(|value| has_value(value))
        .or_else(|| {
            deployment
                .bucket_name
                .as_ref()
                .filter(|value| has_value(value))
        });
    let bucket_name = match bucket {
        Some(Json::Text(name)) => Some(name.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => environment_text(environment.batch_bucket_name.as_ref())
            .or_else(|| environment_text(environment.bucket_name.as_ref())),
    };
    let credentials = deployment
        .vertex_credentials
        .as_ref()
        .filter(|value| has_value(value))
        .or_else(|| argument.filter(|value| has_value(value)));
    let service_account = match credentials {
        Some(Json::Object(fields)) => Some(SecretString::from(dumps_object(fields))),
        Some(Json::Text(text)) => Some(SecretString::from(text.clone())),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_)) | None => environment
            .path_service_account
            .as_ref()
            .filter(|secret| !secret.expose_secret().is_empty())
            .cloned(),
    };
    ReadSettings {
        bucket_name,
        service_account,
    }
}
