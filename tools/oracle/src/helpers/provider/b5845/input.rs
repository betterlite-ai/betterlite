use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use betterlite_openai::embedding::EmbeddingInput;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_provider::sap_auth::environment::SapEnvironment;
use secrecy::SecretString;
use serde::Deserialize;

pub(crate) const DEFAULT_RESOURCE_GROUP: &str = "default";
const PLACEHOLDER_TOKEN: &str = "Bearer no-token";
const DEFAULT_HOME: &str = "/nonexistent-home";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelInput {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) family: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatRequestInput {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) messages: Vec<Json>,
    #[serde(default)]
    pub(crate) optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatResponseInput {
    pub(crate) model: String,
    pub(crate) raw_text: String,
    #[serde(default)]
    pub(crate) messages: Vec<Json>,
    pub(crate) api_key: Option<String>,
    pub(crate) request_data: Option<Json>,
    pub(crate) response_format: Option<Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionInput {
    token: String,
    resource_group: Option<String>,
    deployment_url: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct EnvironmentInput {
    vars: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnvironmentCall {
    pub(crate) service_key: Option<Json>,
    #[serde(default)]
    pub(crate) environment: EnvironmentInput,
    pub(crate) session: Option<SessionInput>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum EmbeddingCall {
    Request {
        model: String,
        input: EmbeddingInput,
        #[serde(default)]
        optional_params: Object,
    },
    MapParams {
        model: String,
        #[serde(default)]
        non_default_params: Object,
        #[serde(default)]
        optional_params: Object,
        #[serde(default)]
        drop_params: bool,
    },
    Prepare {
        session: Option<SessionInput>,
    },
    Response {
        raw_text: String,
    },
    ErrorClass {
        message: String,
        status: u16,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", content = "calls", rename_all = "snake_case")]
pub(crate) enum Input {
    SupportedParams(Vec<ModelInput>),
    Requests(Vec<ChatRequestInput>),
    Responses(Vec<ChatResponseInput>),
    Environments(Vec<EnvironmentCall>),
    Embeddings(Vec<EmbeddingCall>),
}

// A call without a session only runs the credential failure paths, where these values are never read
pub(crate) fn session_parts(session: Option<&SessionInput>) -> (SecretString, String, String) {
    (
        SecretString::from(session.map_or(PLACEHOLDER_TOKEN, |fields| fields.token.as_str())),
        session
            .and_then(|fields| fields.resource_group.clone())
            .unwrap_or_else(|| String::from(DEFAULT_RESOURCE_GROUP)),
        session.map_or_else(String::new, |fields| fields.deployment_url.clone()),
    )
}

pub(crate) struct FixtureEnvironment<'input> {
    pub(crate) input: &'input EnvironmentInput,
}

impl SapEnvironment for FixtureEnvironment<'_> {
    fn var(&self, name: &str) -> Option<String> {
        self.input.vars.get(name).cloned()
    }

    fn file_exists(&self, _path: &Path) -> bool {
        false
    }

    fn read_file(&self, path: &Path) -> Result<String, io::Error> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            path.display().to_string(),
        ))
    }

    fn home_dir(&self) -> PathBuf {
        PathBuf::from(DEFAULT_HOME)
    }
}
