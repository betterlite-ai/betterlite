use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use betterlite_prompts::arguments::json::Json;
use betterlite_provider::sap_auth::credentials::{
    Credentials, fetch_credentials, validate_credentials,
};
use betterlite_provider::sap_auth::deployment::{
    DeploymentConfigurations, DeploymentMode, orchestration_headers, pick_orchestration_deployment,
};
use betterlite_provider::sap_auth::environment::SapEnvironment;
use betterlite_provider::sap_auth::error::{ConfigurationLookupError, SapAuthError};
use betterlite_provider::sap_auth::token::{
    ClientIdentity, SapClock, SapTokenTransport, TokenCache, TokenRequest, TokenTransportError,
    build_token_request,
};
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::helpers::HelperError;

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
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialsInput {
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    client_secret: Option<String>,
    #[serde(default)]
    auth_url: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    cert_file_path: Option<String>,
    #[serde(default)]
    key_file_path: Option<String>,
    #[serde(default)]
    cert_str: Option<String>,
    #[serde(default)]
    key_str: Option<String>,
    #[serde(default)]
    resource_group: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ModeInput {
    Chat,
    Embedding,
}

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
    TokenRequest {
        credentials: CredentialsInput,
    },
    Token {
        credentials: CredentialsInput,
        responses: Vec<String>,
        times: Vec<i64>,
    },
    Deployment {
        mode: ModeInput,
        listing: Json,
        configurations: BTreeMap<String, Json>,
    },
    Headers {
        token: String,
        resource_group: String,
    },
}

impl From<&CredentialsInput> for Credentials {
    fn from(input: &CredentialsInput) -> Self {
        Self {
            client_id: input.client_id.clone(),
            client_secret: input.client_secret.clone().map(SecretString::from),
            auth_url: input.auth_url.clone(),
            base_url: input.base_url.clone(),
            cert_file_path: input.cert_file_path.clone(),
            key_file_path: input.key_file_path.clone(),
            cert_str: input.cert_str.clone(),
            key_str: input.key_str.clone().map(SecretString::from),
            resource_group: input
                .resource_group
                .clone()
                .unwrap_or_else(|| "default".to_owned()),
        }
    }
}

struct MapEnvironment {
    vars: BTreeMap<String, String>,
    files: BTreeMap<PathBuf, String>,
    home: PathBuf,
}

impl MapEnvironment {
    fn from_input(input: &EnvironmentInput) -> Self {
        Self {
            vars: input.vars.clone(),
            files: input
                .files
                .iter()
                .map(|(path, contents)| (PathBuf::from(path), contents.clone()))
                .collect(),
            home: PathBuf::from(input.home.as_str()),
        }
    }
}

impl SapEnvironment for MapEnvironment {
    fn var(&self, name: &str) -> Option<String> {
        self.vars.get(name).cloned()
    }

    fn file_exists(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }

    fn read_file(&self, path: &Path) -> Result<String, io::Error> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    fn home_dir(&self) -> PathBuf {
        self.home.clone()
    }
}

struct ScriptedClock {
    moments: Vec<DateTime<Utc>>,
    index: Mutex<usize>,
}

impl ScriptedClock {
    fn new(times: &[i64]) -> Result<Self, Box<HelperError>> {
        let moments = times
            .iter()
            .map(|seconds| {
                DateTime::from_timestamp(*seconds, 0).ok_or_else(|| {
                    input_failure(format!("the timestamp {seconds} is out of range"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            moments,
            index: Mutex::new(0),
        })
    }
}

impl SapClock for ScriptedClock {
    fn now(&self) -> DateTime<Utc> {
        let mut index = self.index.lock().unwrap_or_else(PoisonError::into_inner);
        let position = *index;
        *index = position.saturating_add(1);
        drop(index);
        self.moments
            .get(position)
            .copied()
            .unwrap_or(DateTime::UNIX_EPOCH)
    }
}

struct Scripted {
    responses: VecDeque<String>,
    requests: usize,
}

struct ScriptedTransport {
    script: Mutex<Scripted>,
}

impl ScriptedTransport {
    fn new(responses: &[String]) -> Self {
        Self {
            script: Mutex::new(Scripted {
                responses: responses.iter().cloned().collect(),
                requests: 0,
            }),
        }
    }

    fn request_count(&self) -> usize {
        self.script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .requests
    }
}

impl SapTokenTransport for ScriptedTransport {
    fn post(&self, _request: &TokenRequest) -> Result<String, TokenTransportError> {
        let mut script = self.script.lock().unwrap_or_else(PoisonError::into_inner);
        script.requests = script.requests.saturating_add(1);
        script
            .responses
            .pop_front()
            .ok_or_else(|| TokenTransportError {
                text: "the case scripted no more token responses".to_owned(),
            })
    }
}

struct MapConfigurations<'input> {
    configurations: &'input BTreeMap<String, Json>,
}

impl DeploymentConfigurations for MapConfigurations<'_> {
    fn configuration(&self, configuration_id: &str) -> Result<Json, ConfigurationLookupError> {
        self.configurations
            .get(configuration_id)
            .cloned()
            .ok_or_else(|| ConfigurationLookupError {
                message: format!("the case has no configuration {configuration_id}"),
            })
    }
}

fn input_failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn auth_failure(error: &SapAuthError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "SapAuthError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn insert_text(
    output: &mut serde_json::Map<String, serde_json::Value>,
    name: &str,
    value: Option<&String>,
) {
    if let Some(text) = value {
        output.insert(name.to_owned(), serde_json::json!(text));
    }
}

fn insert_secret(
    output: &mut serde_json::Map<String, serde_json::Value>,
    name: &str,
    value: Option<&SecretString>,
) {
    if let Some(secret) = value {
        output.insert(name.to_owned(), serde_json::json!(secret.expose_secret()));
    }
}

fn credentials_output(
    environment: &EnvironmentInput,
    service_key: Option<&Json>,
    settings: Option<&Json>,
) -> crate::helpers::HelperResult {
    let credentials = fetch_credentials(
        &MapEnvironment::from_input(environment),
        service_key,
        settings,
    )
    .map_err(|error| auth_failure(&error))?;
    let mut output = serde_json::Map::new();
    insert_text(&mut output, "client_id", credentials.client_id.as_ref());
    insert_secret(
        &mut output,
        "client_secret",
        credentials.client_secret.as_ref(),
    );
    insert_text(&mut output, "auth_url", credentials.auth_url.as_ref());
    insert_text(&mut output, "base_url", credentials.base_url.as_ref());
    insert_text(
        &mut output,
        "cert_file_path",
        credentials.cert_file_path.as_ref(),
    );
    insert_text(
        &mut output,
        "key_file_path",
        credentials.key_file_path.as_ref(),
    );
    insert_text(&mut output, "cert_str", credentials.cert_str.as_ref());
    insert_secret(&mut output, "key_str", credentials.key_str.as_ref());
    output.insert(
        "resource_group".to_owned(),
        serde_json::json!(credentials.resource_group),
    );
    Ok(serde_json::Value::Object(output))
}

fn validate_output(input: &CredentialsInput) -> crate::helpers::HelperResult {
    validate_credentials(&Credentials::from(input)).map_err(|error| auth_failure(&error))?;
    Ok(serde_json::Value::Null)
}

fn token_request_output(input: &CredentialsInput) -> crate::helpers::HelperResult {
    let request =
        build_token_request(&Credentials::from(input)).map_err(|error| auth_failure(&error))?;
    let mut form = serde_json::Map::new();
    form.insert(
        "grant_type".to_owned(),
        serde_json::json!(request.form.grant_type),
    );
    form.insert(
        "client_id".to_owned(),
        serde_json::json!(request.form.client_id),
    );
    if let Some(client_secret) = &request.form.client_secret {
        form.insert(
            "client_secret".to_owned(),
            serde_json::json!(client_secret.expose_secret()),
        );
    }
    let identity = match request.identity {
        ClientIdentity::None => serde_json::json!({"kind": "none"}),
        ClientIdentity::Pem { cert, key } => {
            serde_json::json!({"kind": "pem", "cert": cert, "key": key.expose_secret()})
        }
        ClientIdentity::Files {
            cert_path,
            key_path,
        } => serde_json::json!({"kind": "files", "cert": cert_path, "key": key_path}),
    };
    Ok(serde_json::json!({"url": request.url, "form": form, "identity": identity}))
}

fn token_output(
    input: &CredentialsInput,
    responses: &[String],
    times: &[i64],
) -> crate::helpers::HelperResult {
    let cache = TokenCache::new(&Credentials::from(input)).map_err(|error| auth_failure(&error))?;
    let clock = ScriptedClock::new(times)?;
    let transport = ScriptedTransport::new(responses);
    let mut tokens = Vec::with_capacity(times.len());
    for _ in times {
        tokens.push(
            cache
                .token(&clock, &transport)
                .map_err(|error| auth_failure(&error))?,
        );
    }
    Ok(serde_json::json!({"tokens": tokens, "requests": transport.request_count()}))
}

fn deployment_output(
    mode: &ModeInput,
    listing: &Json,
    configurations: &BTreeMap<String, Json>,
) -> crate::helpers::HelperResult {
    let deployment_mode = match mode {
        ModeInput::Chat => DeploymentMode::Chat,
        ModeInput::Embedding => DeploymentMode::Embedding,
    };
    let source = MapConfigurations { configurations };
    let url = pick_orchestration_deployment(listing, deployment_mode, &source)
        .map_err(|error| auth_failure(&error))?;
    Ok(serde_json::json!(url))
}

fn headers_output(token: &str, resource_group: &str) -> crate::helpers::HelperResult {
    let headers =
        orchestration_headers(token, resource_group).map_err(|error| auth_failure(&error))?;
    let mut output = serde_json::Map::new();
    for (name, value) in &headers {
        let text = value
            .to_str()
            .map_err(|error| input_failure(error.to_string()))?;
        output.insert(name.as_str().to_owned(), serde_json::json!(text));
    }
    Ok(serde_json::Value::Object(output))
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
            Input::TokenRequest { credentials } => token_request_output(credentials),
            Input::Token {
                credentials,
                responses,
                times,
            } => token_output(credentials, responses, times),
            Input::Deployment {
                mode,
                listing,
                configurations,
            } => deployment_output(mode, listing, configurations),
            Input::Headers {
                token,
                resource_group,
            } => headers_output(token, resource_group),
        }
    }
}
