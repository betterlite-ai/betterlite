use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

pub const VALID_ENVIRONMENTS: [&str; 3] = ["development", "staging", "production"];

// The ValueError texts of active_in_environment, with the deployment's model_info kept as it was read.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EnvironmentError {
    #[error(
        "supported_environments must be a list of ['development', 'staging', 'production']. but set as: {value} for model_info: {model_info}"
    )]
    NotList { value: String, model_info: String },
    #[error("Set 'supported_environments' for model but not 'LITELLM_ENVIRONMENT' set in .env")]
    MissingEnvironment,
    #[error(
        "LITELLM_ENVIRONMENT must be one of ['development', 'staging', 'production']. but set as: {value}"
    )]
    UnknownEnvironment { value: String },
    #[error(
        "supported_environments must be one of ['development', 'staging', 'production']. but set as: {value} for model_info: {model_info}"
    )]
    UnknownSupportedEnvironment { value: String, model_info: String },
}

fn listing(value: &Json) -> String {
    match serde_json::to_string(value) {
        Ok(text) => text,
        Err(_error) => String::new(),
    }
}

// active_in_environment: a deployment with supported_environments loads only where LITELLM_ENVIRONMENT is listed.
pub fn active_in_environment(
    model_info: &JsonObject,
    environment: Option<&str>,
) -> Result<bool, EnvironmentError> {
    let supported = match model_info.get("supported_environments") {
        None | Some(Json::Null) => return Ok(true),
        Some(Json::List(entries)) => entries,
        Some(other) => {
            return Err(EnvironmentError::NotList {
                value: listing(other),
                model_info: listing(&Json::Object(model_info.clone())),
            });
        }
    };
    let Some(environment) = environment else {
        return Err(EnvironmentError::MissingEnvironment);
    };
    if !VALID_ENVIRONMENTS.contains(&environment) {
        return Err(EnvironmentError::UnknownEnvironment {
            value: environment.to_owned(),
        });
    }
    for entry in supported {
        match entry {
            Json::Text(name) if VALID_ENVIRONMENTS.contains(&name.as_str()) => {}
            other => {
                return Err(EnvironmentError::UnknownSupportedEnvironment {
                    value: listing(other),
                    model_info: listing(&Json::Object(model_info.clone())),
                });
            }
        }
    }
    Ok(supported
        .iter()
        .any(|entry| matches!(entry, Json::Text(name) if name == environment)))
}
