use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::StandardCallbackDynamicParams;
use indexmap::IndexMap;
use secrecy::SecretString;

pub const DEFAULT_PROJECT: &str = "litellm-completion";
pub const DEFAULT_BASE_URL: &str = "https://api.smith.langchain.com";

#[derive(Clone, Debug, Default)]
pub struct CredentialParams {
    pub api_key: Option<SecretString>,
    pub project: Option<String>,
    pub base_url: Option<String>,
    pub tenant_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Credentials {
    pub api_key: Option<SecretString>,
    pub project: String,
    pub base_url: String,
    pub tenant_id: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("standard_callback_dynamic_params must be an object")]
    ParamsType,
    #[error("standard_callback_dynamic_params.{field} must be a string")]
    FieldType { field: &'static str },
}

// LangsmithLogger.get_credentials_from_env; an explicit base URL with allow_env_credentials off keeps every env var out.
#[must_use]
pub fn get_credentials_from_env(
    environment: &dyn EnvironmentLookup,
    params: &CredentialParams,
    allow_env_credentials: bool,
) -> Credentials {
    let isolated = !allow_env_credentials && params.base_url.is_some();
    // An empty environment variable counts as unset.
    let read = |name: &str| {
        if isolated {
            None
        } else {
            environment.lookup(name).filter(|value| !value.is_empty())
        }
    };
    Credentials {
        api_key: params
            .api_key
            .clone()
            .or_else(|| read("LANGSMITH_API_KEY").map(SecretString::from)),
        project: params
            .project
            .clone()
            .or_else(|| read("LANGSMITH_PROJECT"))
            .unwrap_or_else(|| DEFAULT_PROJECT.to_owned()),
        base_url: params
            .base_url
            .clone()
            .or_else(|| read("LANGSMITH_BASE_URL"))
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_owned()),
        tenant_id: params
            .tenant_id
            .clone()
            .or_else(|| read("LANGSMITH_TENANT_ID")),
    }
}

// LangsmithLogger._get_credentials_to_use_for_request; the dynamic params recompute from env unless they carry a base URL.
pub fn credentials_for_request(
    environment: &dyn EnvironmentLookup,
    default_credentials: &Credentials,
    dynamic_params: Option<&StandardCallbackDynamicParams>,
) -> Result<Credentials, CredentialError> {
    let Some(dynamic) = dynamic_params else {
        return Ok(default_credentials.clone());
    };
    let Json::Object(fields) = dynamic else {
        return Err(CredentialError::ParamsType);
    };
    let params = CredentialParams {
        api_key: credential_field(fields, "langsmith_api_key")?.map(SecretString::from),
        project: credential_field(fields, "langsmith_project")?,
        base_url: credential_field(fields, "langsmith_base_url")?,
        tenant_id: credential_field(fields, "langsmith_tenant_id")?,
    };
    let allow_env_credentials = params.base_url.is_none();
    Ok(get_credentials_from_env(
        environment,
        &params,
        allow_env_credentials,
    ))
}

fn credential_field(
    fields: &IndexMap<String, Json>,
    name: &'static str,
) -> Result<Option<String>, CredentialError> {
    match fields.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(CredentialError::FieldType { field: name })
        }
    }
}
