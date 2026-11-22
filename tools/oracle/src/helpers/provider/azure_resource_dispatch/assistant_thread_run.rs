mod definitions;
mod messages;
mod runs;
mod threads;

use std::collections::BTreeMap;

use betterlite_azure::azure_assistant_definitions::operations::ClientRequest;
use betterlite_azure::azure_urls::AuthParams;
use betterlite_azure::endpoints::projection::{LiteLlmParams, SessionKind};
use betterlite_config::sections::azure_endpoints::Settings;
use betterlite_oracle_helper_api::helper::HelperResult;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;

use super::support::{Answer, NoSources};

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub(super) steps: Vec<Step>,
}

// One B686, B687, B697 or B543 call per step, each served by the same fake transport
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Step {
    DefinitionsCreate(Reply),
    DefinitionsList(Reply),
    DefinitionsDelete(Reply),
    ThreadCreate(Reply),
    ThreadGet(Reply),
    MessageCreate(Reply),
    MessageList(Reply),
    RunPoll(RunFixture),
    RunStream(RunFixture),
}

#[derive(Debug, Deserialize)]
pub(crate) struct Reply {
    pub(super) api_base: String,
    pub(super) api_version: String,
    pub(super) api_key: String,
    pub(super) status: u16,
    pub(super) body: String,
    #[serde(default)]
    pub(super) target_id: String,
}

// A polled run queues its create and each poll, a streamed one a single event body
#[derive(Debug, Deserialize)]
pub(crate) struct RunFixture {
    pub(super) api_base: String,
    pub(super) api_version: String,
    pub(super) api_key: String,
    pub(super) replies: Vec<Planned>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Planned {
    pub(super) status: u16,
    pub(super) body: String,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
}

impl Planned {
    pub(super) fn planned(&self) -> super::support::Reply {
        let mut reply = super::support::Reply::json(self.status, &self.body);
        for (name, value) in &self.headers {
            let parsed_name = http::HeaderName::from_bytes(name.as_bytes());
            let parsed_value = http::HeaderValue::from_str(value);
            if let (Ok(header), Ok(text)) = (parsed_name, parsed_value) {
                reply.headers.insert(header, text);
            }
        }
        reply
    }
}

// The facade request every assistant leaf reads, built from the step's own Azure settings
pub(super) struct Fixture {
    settings: Settings,
    environment: BTreeMap<String, String>,
    sources: NoSources,
    params: LiteLlmParams,
    api_key: Option<SecretString>,
    api_base: Option<String>,
    api_version: Option<String>,
}

impl Fixture {
    pub(super) fn new(api_key: &str, api_base: &str, api_version: &str) -> Self {
        Self {
            settings: Settings {
                default_max_retries: 2,
                azure_default_api_version: String::from("2025-02-01-preview"),
            },
            environment: BTreeMap::new(),
            sources: NoSources,
            params: LiteLlmParams {
                azure_ad_token: None,
                auth: AuthParams::default(),
                max_retries: None,
                timeout: None,
            },
            api_key: Some(SecretString::from(api_key.to_owned())),
            api_base: Some(api_base.to_owned()),
            api_version: Some(api_version.to_owned()),
        }
    }

    pub(super) fn request(&self) -> ClientRequest<'_> {
        ClientRequest {
            params: self.params.clone(),
            provider: None,
            api_key: self.api_key.clone(),
            api_base: self.api_base.clone(),
            api_version: self.api_version.clone(),
            azure_ad_token: None,
            timeout: 600.0,
            max_retries: None,
            refresh_enabled: false,
            session: SessionKind::Dedicated,
            settings: &self.settings,
            environment: &self.environment,
            sources: &self.sources,
        }
    }
}

pub(super) fn run(input: &Input) -> HelperResult {
    let mut answers = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        let (kind, value) = match step {
            Step::DefinitionsCreate(_) | Step::DefinitionsList(_) | Step::DefinitionsDelete(_) => {
                definitions::answer(step)?
            }
            Step::ThreadCreate(_) | Step::ThreadGet(_) => threads::answer(step)?,
            Step::MessageCreate(_) | Step::MessageList(_) => messages::answer(step)?,
            Step::RunPoll(_) | Step::RunStream(_) => runs::answer(step)?,
        };
        answers.push(json!({"kind": kind, "value": value}));
    }
    Ok(json!({"steps": answers}))
}
