use std::fs;
use std::path::Path;

use betterlite_postgres_fixture::fixtures::http_scenarios::SetupJournalEntry;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde_json::Value;

use super::compare::{Difference, Side, side};
use super::error::ScenarioError;
use super::model::{HttpScenario, POLLING_KEY_PREFIX, POLLING_TTL_SECONDS};
use super::run::PeerTranscript;
use crate::http::Response;
use crate::mock::ProviderRequest;
use crate::paths::{safe_name, unique_id};

#[derive(Clone, Copy)]
pub(crate) struct ArtifactInput<'input> {
    pub(crate) root: &'input Path,
    pub(crate) scenario: &'input HttpScenario,
    pub(crate) first: &'input PeerTranscript,
    pub(crate) second: &'input PeerTranscript,
    pub(crate) difference: Option<&'input Difference>,
    pub(crate) secrets: &'input [SecretString],
}

#[derive(Serialize)]
struct RedisView {
    key: String,
    ttl_seconds: u64,
}

#[derive(Serialize)]
struct JournalView<'input> {
    rows: &'input [SetupJournalEntry],
    redis: Vec<RedisView>,
}

#[derive(Serialize)]
struct StepView<'input> {
    index: usize,
    name: &'input str,
    litellm: Side<'input>,
    betterlite: Side<'input>,
}

pub(crate) fn write(input: ArtifactInput<'_>) -> Result<(), ScenarioError> {
    let directory = input
        .root
        .join(safe_name(&input.scenario.name))
        .join(unique_id().map_err(|source| ScenarioError::Request(Box::new(source)))?);
    fs::create_dir_all(&directory).map_err(|source| ScenarioError::Artifact {
        path: directory.clone(),
        source,
    })?;
    let journal = JournalView {
        rows: &input.first.journal,
        redis: redis_views(input.scenario),
    };
    write_value(
        &directory.join("setup-journal.json"),
        &journal,
        input.secrets,
    )?;
    let steps = step_views(input.scenario, input.first, input.second);
    write_value(&directory.join("steps.json"), &steps, input.secrets)?;
    write_side(&directory, "litellm", input.first, input.secrets)?;
    write_side(&directory, "betterlite", input.second, input.secrets)?;
    write_value(
        &directory.join("diff.json"),
        &input.difference,
        input.secrets,
    )?;
    write_log(
        &directory.join("litellm.log"),
        &input.first.log,
        input.secrets,
    )?;
    write_log(
        &directory.join("betterlite.log"),
        &input.second.log,
        input.secrets,
    )
}

fn redis_views(scenario: &HttpScenario) -> Vec<RedisView> {
    scenario
        .setup
        .polling
        .iter()
        .map(|entry| RedisView {
            key: format!("{POLLING_KEY_PREFIX}{}", entry.name),
            ttl_seconds: POLLING_TTL_SECONDS,
        })
        .collect()
}

fn step_views<'input>(
    scenario: &'input HttpScenario,
    first: &'input PeerTranscript,
    second: &'input PeerTranscript,
) -> Vec<StepView<'input>> {
    let mut steps = Vec::new();
    for (index, step) in scenario.steps.iter().enumerate() {
        let (Some(first_capture), Some(second_capture)) =
            (first.steps.get(index), second.steps.get(index))
        else {
            continue;
        };
        steps.push(StepView {
            index,
            name: &step.name,
            litellm: side(first_capture),
            betterlite: side(second_capture),
        });
    }
    steps
}

fn write_side(
    directory: &Path,
    label: &str,
    transcript: &PeerTranscript,
    secrets: &[SecretString],
) -> Result<(), ScenarioError> {
    let responses = transcript
        .steps
        .iter()
        .map(|step| &step.response)
        .collect::<Vec<&Response>>();
    write_value(
        &directory.join(format!("client-{label}.json")),
        &responses,
        secrets,
    )?;
    let requests = transcript
        .steps
        .iter()
        .flat_map(|step| &step.provider)
        .collect::<Vec<&ProviderRequest>>();
    write_value(
        &directory.join(format!("provider-{label}.json")),
        &requests,
        secrets,
    )
}

fn write_value<Data>(
    path: &Path,
    data: &Data,
    secrets: &[SecretString],
) -> Result<(), ScenarioError>
where
    Data: Serialize + ?Sized,
{
    let mut value = serde_json::to_value(data).map_err(ScenarioError::Json)?;
    redact_value(&mut value, secrets);
    let text = serde_json::to_string_pretty(&value).map_err(ScenarioError::Json)?;
    fs::write(path, text).map_err(|source| ScenarioError::Artifact {
        path: path.to_owned(),
        source,
    })
}

fn write_log(path: &Path, log: &str, secrets: &[SecretString]) -> Result<(), ScenarioError> {
    fs::write(path, redact_text(log, secrets)).map_err(|source| ScenarioError::Artifact {
        path: path.to_owned(),
        source,
    })
}

fn redact_text(text: &str, secrets: &[SecretString]) -> String {
    let mut redacted = text.to_owned();
    for secret in secrets {
        let raw = secret.expose_secret();
        if raw.is_empty() {
            continue;
        }
        redacted = redacted.replace(raw, "[REDACTED]");
    }
    redacted
}

fn redact_value(value: &mut Value, secrets: &[SecretString]) {
    match value {
        Value::String(text) => *text = redact_text(text, secrets),
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| redact_value(item, secrets)),
        Value::Object(fields) => fields
            .values_mut()
            .for_each(|item| redact_value(item, secrets)),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}
