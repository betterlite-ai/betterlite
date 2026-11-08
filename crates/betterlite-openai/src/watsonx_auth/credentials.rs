use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use super::environment::{WatsonxEnvironment, first_set};
use super::error::WatsonxError;
use super::types::{WatsonxApiParams, WatsonxCredentials, WatsonxParams};

const KEY_NOT_SET: &str = "Error: Watsonx API key not set. Set WATSONX_API_KEY in environment variables or pass in as parameter - 'api_key='.";
const BASE_NOT_SET: &str = "Error: Watsonx API base not set. Set WATSONX_API_BASE in environment variables or pass in as parameter - 'api_base='.";
const IDS_NOT_SET: &str = "Error: Watsonx project_id and space_id not set. Set WX_PROJECT_ID or WX_SPACE_ID in environment variables or pass in as parameter.";

fn take_text(
    params: &mut WatsonxParams,
    key: &'static str,
) -> Result<Option<String>, WatsonxError> {
    match params.remove(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_)) => {
            Err(WatsonxError::InvalidField { field: key })
        }
    }
}

// dict.pop(outer, dict.pop(inner, None)) evaluates the inner pop first and keeps a present outer key
fn pop_pair(
    params: &mut WatsonxParams,
    outer: &'static str,
    inner: &'static str,
) -> Result<Option<String>, WatsonxError> {
    let inner_value = take_text(params, inner)?;
    params
        .remove(outer)
        .map_or(Ok(inner_value), |value| match value {
            Value::Null => Ok(None),
            Value::String(text) => Ok(Some(text)),
            Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => {
                Err(WatsonxError::InvalidField { field: outer })
            }
        })
}

fn require_ids(
    project_id: Option<&str>,
    space_id: Option<&str>,
    model: Option<&str>,
) -> Result<(), WatsonxError> {
    let needs_ids = model.is_some_and(|name| !name.starts_with("deployment/"));
    if project_id.is_none() && space_id.is_none() && needs_ids {
        return Err(WatsonxError::Api {
            status_code: 401,
            message: IDS_NOT_SET.to_owned(),
            headers: None,
        });
    }
    Ok(())
}

fn api_params_from_environment(
    project_param: Option<String>,
    space_param: Option<String>,
    region_param: Option<String>,
    environment: &WatsonxEnvironment,
) -> WatsonxApiParams {
    let project_id = project_param.or_else(|| {
        first_set(&[
            environment.watsonx_project_id.as_deref(),
            environment.wx_project_id.as_deref(),
            environment.project_id.as_deref(),
        ])
        .map(str::to_owned)
    });
    let space_id = space_param.or_else(|| {
        first_set(&[
            environment.watsonx_deployment_space_id.as_deref(),
            environment.watsonx_space_id.as_deref(),
            environment.wx_space_id.as_deref(),
            environment.space_id.as_deref(),
        ])
        .map(str::to_owned)
    });
    let region_name = region_param.or_else(|| {
        first_set(&[
            environment.watsonx_region.as_deref(),
            environment.wx_region.as_deref(),
            environment.region.as_deref(),
        ])
        .map(str::to_owned)
    });
    WatsonxApiParams {
        project_id,
        space_id,
        region_name,
    }
}

pub fn get_api_params(
    params: &mut WatsonxParams,
    model: Option<&str>,
    environment: &WatsonxEnvironment,
) -> Result<WatsonxApiParams, WatsonxError> {
    let project_param = pop_pair(params, "project_id", "watsonx_project")?;
    let space_param = take_text(params, "space_id")?;
    let region_param = match pop_pair(params, "region_name", "region")? {
        Some(region) => Some(region),
        None => pop_pair(params, "watsonx_region_name", "watsonx_region")?,
    };
    let found = api_params_from_environment(project_param, space_param, region_param, environment);
    require_ids(
        found.project_id.as_deref(),
        found.space_id.as_deref(),
        model,
    )?;
    Ok(found)
}

fn environment_api_key(environment: &WatsonxEnvironment) -> Option<Value> {
    let candidates = [
        environment.watsonx_apikey.as_ref(),
        environment.watsonx_api_key.as_ref(),
        environment.wx_api_key.as_ref(),
        environment.watsonx_zenapikey.as_ref(),
    ];
    for candidate in candidates.into_iter().flatten() {
        let text = candidate.expose_secret();
        if !text.is_empty() {
            return Some(Value::String(text.to_owned()));
        }
    }
    None
}

fn credential_key(
    params: &mut WatsonxParams,
    api_key: Option<&SecretString>,
    environment: &WatsonxEnvironment,
) -> Option<Value> {
    api_key
        .filter(|candidate| !candidate.expose_secret().is_empty())
        .map(|candidate| Value::String(candidate.expose_secret().to_owned()))
        .or_else(|| {
            params
                .remove("apikey")
                .filter(|value| !matches!(value, Value::String(text) if text.is_empty()))
        })
        .or_else(|| environment_api_key(environment))
}

fn credential_base(
    params: &mut WatsonxParams,
    api_base: Option<&str>,
    environment: &WatsonxEnvironment,
) -> Option<Value> {
    if let Some(candidate) = api_base.filter(|candidate| !candidate.is_empty()) {
        return Some(Value::String(candidate.to_owned()));
    }
    let base_url = params.remove("base_url");
    let legacy = params.remove("api_base").or(base_url);
    params
        .remove("url")
        .or(legacy)
        .filter(|value| !matches!(value, Value::String(text) if text.is_empty()))
        .or_else(|| {
            environment
                .api_base()
                .map(|url| Value::String(url.to_owned()))
        })
}

struct CredentialOverrides {
    api_base: Option<Value>,
    api_key: Option<Value>,
    token: Option<Value>,
}

fn credential_overrides(
    params: &mut WatsonxParams,
) -> Result<Option<CredentialOverrides>, WatsonxError> {
    let value = params
        .remove("wx_credentials")
        .map_or_else(|| params.remove("watsonx_credentials"), Some);
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_)) => {
            Err(WatsonxError::InvalidField {
                field: "wx_credentials",
            })
        }
        Some(Value::Object(fields)) => Ok(Some(CredentialOverrides {
            api_base: fields.get("url").cloned(),
            api_key: fields
                .get("apikey")
                .or_else(|| fields.get("api_key"))
                .cloned(),
            token: fields
                .get("token")
                .or_else(|| fields.get("watsonx_token"))
                .cloned(),
        })),
    }
}

pub fn get_watsonx_credentials(
    params: &mut WatsonxParams,
    api_key: Option<&SecretString>,
    api_base: Option<&str>,
    environment: &WatsonxEnvironment,
) -> Result<WatsonxCredentials, WatsonxError> {
    let key_value = credential_key(params, api_key, environment);
    let base_value = credential_base(params, api_base, environment);
    let (final_key, final_base, final_token) = match credential_overrides(params)? {
        Some(found) => (
            found.api_key.or(key_value),
            found.api_base.or(base_value),
            found.token,
        ),
        None => (key_value, base_value, None),
    };
    let Some(Value::String(key_text)) = final_key else {
        return Err(WatsonxError::Api {
            status_code: 401,
            message: KEY_NOT_SET.to_owned(),
            headers: None,
        });
    };
    let Some(Value::String(base_text)) = final_base else {
        return Err(WatsonxError::Api {
            status_code: 401,
            message: BASE_NOT_SET.to_owned(),
            headers: None,
        });
    };
    let token = match final_token {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(SecretString::from(text)),
        Some(Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_)) => {
            return Err(WatsonxError::InvalidField { field: "token" });
        }
    };
    Ok(WatsonxCredentials {
        api_key: SecretString::from(key_text),
        api_base: base_text,
        token,
    })
}
