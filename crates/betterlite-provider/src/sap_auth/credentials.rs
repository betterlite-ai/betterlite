use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret as _, SecretString};

use self::config::load_config;
use self::text::{lookup, string_value};
use self::transform::{CREDENTIAL_VALUES, transformed};
use self::vcap::{find_vcap_service, load_vcap};
use super::environment::SapEnvironment;
use super::error::SapAuthError;

mod config;
mod text;
mod transform;
mod vcap;

pub const AUTH_ENDPOINT_SUFFIX: &str = "/oauth/token";
pub const CONFIG_FILE_ENV_VAR: &str = "AICORE_CONFIG";
pub const HOME_PATH_ENV_VAR: &str = "AICORE_HOME";
pub const PROFILE_ENV_VAR: &str = "AICORE_PROFILE";
pub const SERVICE_KEY_ENV_VAR: &str = "AICORE_SERVICE_KEY";
pub const VCAP_SERVICES_ENV_VAR: &str = "VCAP_SERVICES";
pub const VCAP_AICORE_SERVICE_NAME: &str = "aicore";
pub const DEFAULT_RESOURCE_GROUP: &str = "default";
const RESOURCE_GROUP_ENV_VAR: &str = "AICORE_RESOURCE_GROUP";
const RESOURCE_GROUP_NAME: &str = "resource_group";
const CREDENTIALS_PATH: &str = "credentials";

#[derive(Debug, Clone)]
pub struct Credentials {
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub auth_url: Option<String>,
    pub base_url: Option<String>,
    pub cert_file_path: Option<String>,
    pub key_file_path: Option<String>,
    pub cert_str: Option<String>,
    pub key_str: Option<SecretString>,
    pub resource_group: String,
}

#[derive(Debug, Default)]
struct Extracted {
    values: BTreeMap<&'static str, String>,
    resource_group: Option<String>,
}

pub fn fetch_credentials(
    environment: &dyn SapEnvironment,
    service_key: Option<&Json>,
    litellm_sap_service_key: Option<&Json>,
) -> Result<Credentials, SapAuthError> {
    let config = load_config(environment)?;
    let vcap = load_vcap(environment);
    let document = select_service_key(environment, [service_key, litellm_sap_service_key])
        .and_then(|value| parse_service_key(&value));
    let sources = [
        document
            .as_ref()
            .map_or_else(Extracted::default, extract_service_key),
        extract_environment(environment),
        extract_config(&config),
        vcap.as_ref()
            .and_then(find_vcap_service)
            .map_or_else(Extracted::default, extract_vcap),
    ];
    let mut resolved = None;
    let mut resource_group = None;
    for source in sources {
        if resolved.is_none() && !source.values.is_empty() {
            resolved = Some(source.values);
        }
        resource_group = resource_group.or(source.resource_group);
    }
    let mut values = resolved.ok_or(SapAuthError::NoCredentials)?;
    if let Some(cert_url) = values.remove("cert_url") {
        values.insert("auth_url", cert_url);
    }
    Ok(Credentials {
        client_id: values.remove("client_id"),
        client_secret: values.remove("client_secret").map(SecretString::from),
        auth_url: values.remove("auth_url"),
        base_url: values.remove("base_url"),
        cert_file_path: values.remove("cert_file_path"),
        key_file_path: values.remove("key_file_path"),
        cert_str: values.remove("cert_str"),
        key_str: values.remove("key_str").map(SecretString::from),
        resource_group: resource_group.unwrap_or_else(|| DEFAULT_RESOURCE_GROUP.to_owned()),
    })
}

pub fn validate_credentials(credentials: &Credentials) -> Result<(), SapAuthError> {
    if credentials.auth_url.as_deref().is_none_or(str::is_empty)
        || credentials.client_id.as_deref().is_none_or(str::is_empty)
        || credentials.base_url.as_deref().is_none_or(str::is_empty)
    {
        return Err(SapAuthError::MissingCredentials);
    }
    let secret = credentials
        .client_secret
        .as_ref()
        .is_some_and(|value| !value.expose_secret().is_empty());
    let pem = credentials
        .cert_str
        .as_deref()
        .is_some_and(|value| !value.is_empty())
        && credentials
            .key_str
            .as_ref()
            .is_some_and(|value| !value.expose_secret().is_empty());
    let files = credentials
        .cert_file_path
        .as_deref()
        .is_some_and(|value| !value.is_empty())
        && credentials
            .key_file_path
            .as_deref()
            .is_some_and(|value| !value.is_empty());
    let modes = [secret, pem, files];
    if modes.into_iter().filter(|enabled| *enabled).count() == 1 {
        Ok(())
    } else {
        Err(SapAuthError::IncompleteCredentials)
    }
}

fn select_service_key(
    environment: &dyn SapEnvironment,
    candidates: [Option<&Json>; 2],
) -> Option<Json> {
    candidates
        .into_iter()
        .flatten()
        .find(|value| match value {
            Json::Null => false,
            Json::Text(text) => !text.is_empty(),
            Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => true,
        })
        .cloned()
        .or_else(|| environment.var(SERVICE_KEY_ENV_VAR).map(Json::Text))
}

fn parse_service_key(value: &Json) -> Option<Json> {
    match value {
        Json::Text(text) => match serde_json::from_str::<Json>(text) {
            Ok(Json::Null) => None,
            Ok(parsed) => Some(parsed),
            Err(_) => {
                tracing::warn!("the SAP service key is not valid JSON; skipping it");
                None
            }
        },
        Json::Object(_) => Some(value.clone()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            tracing::warn!("SAP service key has an unexpected type; ignoring this source");
            None
        }
    }
}

fn extract_with<F>(mut lookup: F) -> Extracted
where
    F: FnMut(&'static str, Option<&'static [&'static str]>) -> Option<String>,
{
    let mut extracted = Extracted::default();
    for (name, vcap_key, transform) in CREDENTIAL_VALUES {
        if let Some(value) = lookup(name, vcap_key) {
            extracted
                .values
                .insert(name, transformed(transform, &value));
        }
    }
    extracted
}

fn extract_service_key(document: &Json) -> Extracted {
    let mut extracted = extract_with(|name, vcap_key| service_key_lookup(document, name, vcap_key));
    extracted.resource_group = service_key_lookup(document, RESOURCE_GROUP_NAME, None);
    extracted
}

fn service_key_lookup(
    document: &Json,
    name: &str,
    vcap_key: Option<&'static [&'static str]>,
) -> Option<String> {
    let mut value =
        lookup(document, &prefixed_path(name, vcap_key)).and_then(|found| string_value(&found));
    if value.is_none()
        && let Some(keys) = vcap_key
    {
        value = lookup(document, keys).and_then(|found| string_value(&found));
    }
    value
}

fn prefixed_path<'name>(
    name: &'name str,
    vcap_key: Option<&'static [&'static str]>,
) -> Vec<&'name str> {
    vcap_key.map_or_else(
        || vec![name],
        |keys| {
            let mut path = vec![CREDENTIALS_PATH];
            path.extend(keys.iter().copied());
            path
        },
    )
}

fn extract_environment(environment: &dyn SapEnvironment) -> Extracted {
    let mut extracted =
        extract_with(|name, _| environment.var(&format!("AICORE_{}", name.to_ascii_uppercase())));
    extracted.resource_group = environment.var(RESOURCE_GROUP_ENV_VAR);
    extracted
}

fn extract_config(config: &Json) -> Extracted {
    let Json::Object(fields) = config else {
        tracing::warn!("the SAP AI Core config file is not a JSON object; it provides no values");
        return Extracted::default();
    };
    let mut extracted = extract_with(|name, _| {
        let variable = format!("AICORE_{}", name.to_ascii_uppercase());
        config_value(fields, &variable, name)
    });
    extracted.resource_group = config_value(fields, RESOURCE_GROUP_ENV_VAR, RESOURCE_GROUP_NAME);
    extracted
}

fn config_value(fields: &Object, variable: &str, name: &str) -> Option<String> {
    let chosen = match fields.get(variable) {
        Some(Json::Null) | None => fields.get(name),
        Some(value) => Some(value),
    };
    let value = chosen?;
    string_value(value)
}

fn extract_vcap(service: &Json) -> Extracted {
    let mut extracted = extract_with(|name, vcap_key| {
        let path = prefixed_path(name, vcap_key);
        let found = lookup(service, &path)?;
        string_value(&found)
    });
    extracted.resource_group =
        lookup(service, &[RESOURCE_GROUP_NAME]).and_then(|found| string_value(&found));
    extracted
}
