use std::collections::BTreeMap;
use std::path::PathBuf;

use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret as _, SecretString};

use super::environment::SapEnvironment;
use super::error::SapAuthError;

pub const AUTH_ENDPOINT_SUFFIX: &str = "/oauth/token";
pub const CONFIG_FILE_ENV_VAR: &str = "AICORE_CONFIG";
pub const HOME_PATH_ENV_VAR: &str = "AICORE_HOME";
pub const PROFILE_ENV_VAR: &str = "AICORE_PROFILE";
pub const SERVICE_KEY_ENV_VAR: &str = "AICORE_SERVICE_KEY";
pub const VCAP_SERVICES_ENV_VAR: &str = "VCAP_SERVICES";
pub const VCAP_AICORE_SERVICE_NAME: &str = "aicore";
pub const DEFAULT_RESOURCE_GROUP: &str = "default";
const RESOURCE_GROUP_ENV_VAR: &str = "AICORE_RESOURCE_GROUP";
const DEFAULT_PROFILE_CONFIG: &str = "config.json";
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

#[derive(Debug, Clone, Copy)]
enum Transform {
    AuthEndpoint,
    ApiBase,
    PemText,
}

impl Transform {
    fn apply(self, value: &str) -> String {
        match self {
            Self::AuthEndpoint => {
                let trimmed = value.trim_end_matches('/');
                let suffix = if value.ends_with(AUTH_ENDPOINT_SUFFIX) {
                    ""
                } else {
                    AUTH_ENDPOINT_SUFFIX
                };
                format!("{trimmed}{suffix}")
            }
            Self::ApiBase => {
                let trimmed = value.trim_end_matches('/');
                let suffix = if value.ends_with("/v2") { "" } else { "/v2" };
                format!("{trimmed}{suffix}")
            }
            Self::PemText => value.replace("\\n", "\n"),
        }
    }
}

struct CredentialValue {
    name: &'static str,
    vcap_key: Option<&'static [&'static str]>,
    transform: Option<Transform>,
}

const RESOURCE_GROUP_VALUE: CredentialValue = CredentialValue {
    name: RESOURCE_GROUP_NAME,
    vcap_key: None,
    transform: None,
};

const CREDENTIAL_VALUES: [CredentialValue; 9] = [
    CredentialValue {
        name: "client_id",
        vcap_key: Some(&["clientid"]),
        transform: None,
    },
    CredentialValue {
        name: "client_secret",
        vcap_key: Some(&["clientsecret"]),
        transform: None,
    },
    CredentialValue {
        name: "auth_url",
        vcap_key: Some(&["url"]),
        transform: Some(Transform::AuthEndpoint),
    },
    CredentialValue {
        name: "base_url",
        vcap_key: Some(&["serviceurls", "AI_API_URL"]),
        transform: Some(Transform::ApiBase),
    },
    CredentialValue {
        name: "cert_url",
        vcap_key: Some(&["certurl"]),
        transform: Some(Transform::AuthEndpoint),
    },
    CredentialValue {
        name: "cert_file_path",
        vcap_key: None,
        transform: None,
    },
    CredentialValue {
        name: "key_file_path",
        vcap_key: None,
        transform: None,
    },
    CredentialValue {
        name: "cert_str",
        vcap_key: Some(&["certificate"]),
        transform: Some(Transform::PemText),
    },
    CredentialValue {
        name: "key_str",
        vcap_key: Some(&["key"]),
        transform: Some(Transform::PemText),
    },
];

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
    let service_key_document =
        select_service_key(environment, service_key, litellm_sap_service_key)
            .and_then(|value| parse_service_key(&value));
    let sources = [
        service_key_document
            .as_ref()
            .map_or_else(Extracted::default, extract_service_key),
        extract_environment(environment),
        extract_config(&config),
        vcap_source(vcap.as_ref()),
    ];
    let mut resolved = None;
    let mut resource_group = None;
    for source in sources {
        if resolved.is_none() && !source.values.is_empty() {
            resolved = Some(source.values);
        }
        if resource_group.is_none() {
            resource_group = source.resource_group;
        }
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
    service_key: Option<&Json>,
    litellm_sap_service_key: Option<&Json>,
) -> Option<Json> {
    let configured = [service_key, litellm_sap_service_key]
        .into_iter()
        .flatten()
        .find(|value| is_selectable(value));
    configured
        .cloned()
        .or_else(|| environment.var(SERVICE_KEY_ENV_VAR).map(Json::Text))
}

const fn is_selectable(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Text(text) => !text.is_empty(),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => true,
    }
}

fn parse_service_key(value: &Json) -> Option<Json> {
    match value {
        Json::Text(text) => match serde_json::from_str::<Json>(text) {
            Ok(Json::Null) => None,
            Ok(parsed) => Some(parsed),
            Err(_) => {
                tracing::warn!(
                    "SAP service key is a string but not valid JSON; skipping this source"
                );
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

fn extract_service_key(document: &Json) -> Extracted {
    let mut extracted = Extracted::default();
    for credential in &CREDENTIAL_VALUES {
        if let Some(value) = service_key_lookup(document, credential) {
            insert_value(&mut extracted.values, credential, &value);
        }
    }
    extracted.resource_group = service_key_lookup(document, &RESOURCE_GROUP_VALUE);
    extracted
}

fn service_key_lookup(document: &Json, credential: &CredentialValue) -> Option<String> {
    let prefixed: Vec<&str> = credential.vcap_key.map_or_else(
        || vec![credential.name],
        |keys| {
            let mut path = vec![CREDENTIALS_PATH];
            path.extend(keys.iter().copied());
            path
        },
    );
    let mut value = lookup(document, &prefixed);
    if value.is_none()
        && let Some(keys) = credential.vcap_key
    {
        value = lookup(document, keys);
    }
    let found = value?;
    string_value(&found)
}

fn extract_environment(environment: &dyn SapEnvironment) -> Extracted {
    let mut extracted = Extracted::default();
    for credential in &CREDENTIAL_VALUES {
        let name = format!("AICORE_{}", credential.name.to_ascii_uppercase());
        if let Some(value) = environment.var(&name) {
            insert_value(&mut extracted.values, credential, &value);
        }
    }
    extracted.resource_group = environment.var(RESOURCE_GROUP_ENV_VAR);
    extracted
}

fn extract_config(config: &Json) -> Extracted {
    let mut extracted = Extracted::default();
    let Json::Object(fields) = config else {
        tracing::warn!(
            "the SAP AI Core config file is not a JSON object, so it provides no values"
        );
        return extracted;
    };
    for credential in &CREDENTIAL_VALUES {
        let env_name = format!("AICORE_{}", credential.name.to_ascii_uppercase());
        let chosen = match fields.get(&env_name) {
            Some(Json::Null) | None => fields.get(credential.name),
            Some(value) => Some(value),
        };
        if let Some(text) = chosen.and_then(string_value) {
            insert_value(&mut extracted.values, credential, &text);
        }
    }
    extracted.resource_group = config_value(fields, RESOURCE_GROUP_ENV_VAR, RESOURCE_GROUP_NAME);
    extracted
}

fn config_value(fields: &Object, env_name: &str, plain_name: &str) -> Option<String> {
    let chosen = match fields.get(env_name) {
        Some(Json::Null) | None => fields.get(plain_name),
        Some(value) => Some(value),
    };
    let value = chosen?;
    string_value(value)
}

fn extract_vcap(service: &Json) -> Extracted {
    let mut extracted = Extracted::default();
    for credential in &CREDENTIAL_VALUES {
        let path: Vec<&str> = credential.vcap_key.map_or_else(
            || vec![credential.name],
            |keys| {
                let mut path = vec![CREDENTIALS_PATH];
                path.extend(keys.iter().copied());
                path
            },
        );
        if let Some(value) = lookup(service, &path).and_then(|found| string_value(&found)) {
            insert_value(&mut extracted.values, credential, &value);
        }
    }
    extracted.resource_group =
        lookup(service, &[RESOURCE_GROUP_NAME]).and_then(|found| string_value(&found));
    extracted
}

fn vcap_source(services: Option<&Object>) -> Extracted {
    services
        .and_then(find_vcap_service)
        .map_or_else(Extracted::default, extract_vcap)
}

fn insert_value(
    values: &mut BTreeMap<&'static str, String>,
    credential: &CredentialValue,
    value: &str,
) {
    let transformed = credential
        .transform
        .map_or_else(|| value.to_owned(), |transform| transform.apply(value));
    values.insert(credential.name, transformed);
}

fn lookup(document: &Json, path: &[&str]) -> Option<Json> {
    let mut current = match document {
        Json::Text(text) => {
            let Ok(parsed) = serde_json::from_str::<Json>(text) else {
                tracing::warn!("SAP service key or VCAP service is a string but not valid JSON");
                return None;
            };
            parsed
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            document.clone()
        }
    };
    for key in path {
        let Some(fields) = current.as_object() else {
            tracing::warn!(
                key = *key,
                "SAP service key or VCAP service traversal hit a non-object value"
            );
            return None;
        };
        current = fields.get(*key)?.clone();
    }
    Some(current)
}

fn string_value(value: &Json) -> Option<String> {
    match value {
        Json::Null => None,
        Json::Text(text) => Some(text.clone()),
        Json::Number(number) => Some(number.to_string()),
        Json::Bool(flag) => Some(flag.to_string()),
        Json::List(_) | Json::Object(_) => {
            tracing::warn!("a SAP AI Core credential value is a JSON array or object; skipping it");
            None
        }
    }
}

fn load_vcap(environment: &dyn SapEnvironment) -> Option<Object> {
    let raw = environment.var(VCAP_SERVICES_ENV_VAR)?;
    if raw.is_empty() {
        return None;
    }
    serde_json::from_str::<Object>(&raw).ok()
}

fn find_vcap_service(services: &Object) -> Option<&Json> {
    for (_, list) in services {
        let Json::List(entries) = list else {
            tracing::warn!("a VCAP_SERVICES service list is not a JSON array; skipping it");
            continue;
        };
        for entry in entries {
            let Json::Object(fields) = entry else {
                continue;
            };
            if fields.get("label").and_then(Json::as_str) == Some(VCAP_AICORE_SERVICE_NAME) {
                return Some(entry);
            }
        }
    }
    None
}

fn load_config(environment: &dyn SapEnvironment) -> Result<Json, SapAuthError> {
    let home = environment
        .var(HOME_PATH_ENV_VAR)
        .map_or_else(|| environment.home_dir().join(".aicore"), PathBuf::from);
    let profile = environment.var(PROFILE_ENV_VAR);
    let config_env = environment.var(CONFIG_FILE_ENV_VAR);
    let explicit_config = config_env.as_deref().is_some_and(|value| !value.is_empty());
    let named_profile = profile
        .as_deref()
        .is_some_and(|value| !value.is_empty() && value != "default");
    let path = match config_env.as_deref() {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => home.join(profile_file_name(profile.as_deref())),
    };
    if environment.file_exists(&path) {
        let text = environment
            .read_file(&path)
            .map_err(|source| SapAuthError::ConfigRead {
                path: path.clone(),
                source,
            })?;
        return serde_json::from_str::<Json>(&text)
            .map_err(|_source| SapAuthError::ConfigNotJson { path });
    }
    if explicit_config || named_profile {
        return Err(SapAuthError::ConfigFileNotFound { path, home });
    }
    Ok(Json::Object(Object::new()))
}

fn profile_file_name(profile: Option<&str>) -> String {
    match profile {
        None | Some("" | "default") => DEFAULT_PROFILE_CONFIG.to_owned(),
        Some(name) => format!("config_{name}.json"),
    }
}
