use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_prompts::arguments::json::Json;
use betterlite_provider::sap_auth::credentials::{
    Credentials, fetch_credentials, validate_credentials,
};
use betterlite_provider::sap_auth::deployment::{DeploymentMode, pick_orchestration_deployment};
use betterlite_provider::sap_auth::token::TokenCache;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use crate::helpers::HelperError;

mod support;

use self::support::{MapConfigurations, Scripted, ScriptedClock, ScriptedTransport};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnvironmentInput {
    #[serde(default)]
    vars: BTreeMap<String, String>,
    #[serde(default)]
    files: BTreeMap<String, String>,
    home: String,
}

#[derive(Debug, Deserialize)]
#[serde(transparent)]
pub(crate) struct CredentialsInput(BTreeMap<String, String>);

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Credentials {
        environment: EnvironmentInput,
        #[serde(default)]
        service_key: Option<Json>,
        #[serde(default)]
        litellm_sap_service_key: Option<Json>,
    },
    Validate {
        credentials: CredentialsInput,
    },
    Token {
        credentials: CredentialsInput,
        responses: Vec<String>,
        times: Vec<i64>,
    },
    Deployment {
        mode: DeploymentMode,
        listing: Json,
        configurations: BTreeMap<String, Json>,
    },
}

#[derive(Serialize)]
struct CredentialsView<'credentials> {
    client_id: Option<&'credentials str>,
    client_secret: Option<&'credentials str>,
    auth_url: Option<&'credentials str>,
    base_url: Option<&'credentials str>,
    cert_file_path: Option<&'credentials str>,
    key_file_path: Option<&'credentials str>,
    cert_str: Option<&'credentials str>,
    key_str: Option<&'credentials str>,
    resource_group: &'credentials str,
}

impl From<&CredentialsInput> for Credentials {
    fn from(input: &CredentialsInput) -> Self {
        let fields = &input.0;
        Self {
            client_id: fields.get("client_id").cloned(),
            client_secret: fields.get("client_secret").cloned().map(SecretString::from),
            auth_url: fields.get("auth_url").cloned(),
            base_url: fields.get("base_url").cloned(),
            cert_file_path: fields.get("cert_file_path").cloned(),
            key_file_path: fields.get("key_file_path").cloned(),
            cert_str: fields.get("cert_str").cloned(),
            key_str: fields.get("key_str").cloned().map(SecretString::from),
            resource_group: fields
                .get("resource_group")
                .cloned()
                .unwrap_or_else(|| "default".to_owned()),
        }
    }
}

fn failure(class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

fn credentials_output(
    environment: &EnvironmentInput,
    service_key: Option<&Json>,
    settings: Option<&Json>,
) -> crate::helpers::HelperResult {
    let credentials = fetch_credentials(environment, service_key, settings)
        .map_err(|error| failure("SapAuthError", &error.to_string()))?;
    let view = CredentialsView {
        client_id: credentials.client_id.as_deref(),
        client_secret: credentials
            .client_secret
            .as_ref()
            .map(ExposeSecret::expose_secret),
        auth_url: credentials.auth_url.as_deref(),
        base_url: credentials.base_url.as_deref(),
        cert_file_path: credentials.cert_file_path.as_deref(),
        key_file_path: credentials.key_file_path.as_deref(),
        cert_str: credentials.cert_str.as_deref(),
        key_str: credentials
            .key_str
            .as_ref()
            .map(ExposeSecret::expose_secret),
        resource_group: &credentials.resource_group,
    };
    serde_json::to_value(view).map_err(|error| failure("InvalidInput", &error.to_string()))
}

fn validate_output(input: &CredentialsInput) -> crate::helpers::HelperResult {
    validate_credentials(&Credentials::from(input))
        .map_err(|error| failure("SapAuthError", &error.to_string()))?;
    Ok(serde_json::Value::Null)
}

fn token_output(
    input: &CredentialsInput,
    responses: &[String],
    times: &[i64],
) -> crate::helpers::HelperResult {
    let cache = TokenCache::new(&Credentials::from(input))
        .map_err(|error| failure("SapAuthError", &error.to_string()))?;
    let clock = ScriptedClock {
        times: Mutex::new(times.iter().copied().collect()),
    };
    let transport = ScriptedTransport {
        script: Mutex::new(Scripted {
            responses: responses.iter().cloned().collect(),
            requests: 0,
        }),
    };
    let mut tokens = Vec::with_capacity(times.len());
    for _ in times {
        tokens.push(
            cache
                .token(&clock, &transport)
                .map_err(|error| failure("SapAuthError", &error.to_string()))?
                .expose_secret()
                .to_owned(),
        );
    }
    let requests = transport
        .script
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .requests;
    Ok(serde_json::json!({"tokens": tokens, "requests": requests}))
}

fn deployment_output(
    mode: DeploymentMode,
    listing: &Json,
    configurations: &BTreeMap<String, Json>,
) -> crate::helpers::HelperResult {
    let source = MapConfigurations { configurations };
    let url = pick_orchestration_deployment(listing, mode, &source)
        .map_err(|error| failure("SapAuthError", &error.to_string()))?;
    Ok(serde_json::json!(url))
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Credentials {
                environment,
                service_key,
                litellm_sap_service_key,
            } => credentials_output(
                environment,
                service_key.as_ref(),
                litellm_sap_service_key.as_ref(),
            ),
            Input::Validate { credentials } => validate_output(credentials),
            Input::Token {
                credentials,
                responses,
                times,
            } => token_output(credentials, responses, times),
            Input::Deployment {
                mode,
                listing,
                configurations,
            } => deployment_output(*mode, listing, configurations),
        }
    }
}
