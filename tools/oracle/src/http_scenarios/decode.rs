use std::collections::{BTreeMap, BTreeSet};

use betterlite_postgres_fixture::fixtures::http_scenarios::ScenarioSetup;
use serde::Deserialize;

use super::error::ScenarioError;
use super::model::{HttpScenario, Limits, Segment, Step, segments};
use crate::case::Ignore;
use crate::multipart_oracle::request::request_body;

const ROOT_FIELDS: [&str; 6] = ["kind", "name", "config", "env", "setup", "steps"];
const RESERVED_ENV: [&str; 2] = ["LITELLM_LOCAL_MODEL_COST_MAP", "LITELLM_MASTER_KEY"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScenarioFile {
    kind: Option<String>,
    name: String,
    config: String,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    setup: ScenarioSetup,
    steps: Vec<Step>,
}

// The closed root is checked before Serde sees it so an unknown or repeated key keeps its own text.
pub(crate) fn validate_root(text: &str) -> Result<(), ScenarioError> {
    let mut seen = BTreeSet::new();
    for key in text.lines().filter_map(root_key) {
        if !ROOT_FIELDS.contains(&key.as_str()) {
            return Err(ScenarioError::UnknownField { field: key });
        }
        if !seen.insert(key.clone()) {
            return Err(ScenarioError::DuplicateField { field: key });
        }
    }
    Ok(())
}

fn root_key(line: &str) -> Option<String> {
    let trimmed = line.trim_end();
    let first = trimmed.chars().next()?;
    if first.is_whitespace()
        || first == '#'
        || first == '-'
        || trimmed.starts_with("---")
        || trimmed.starts_with("...")
    {
        return None;
    }
    let (raw_key, _) = trimmed.split_once(':')?;
    let key = raw_key.trim().trim_matches(['"', '\'']);
    (!key.is_empty()).then(|| key.to_owned())
}

impl ScenarioFile {
    pub(crate) fn into_scenario(self, limits: &Limits) -> Result<HttpScenario, ScenarioError> {
        self.validate(limits)?;
        Ok(HttpScenario {
            name: self.name,
            config: self.config,
            env: self.env,
            setup: self.setup,
            steps: self.steps,
        })
    }

    fn validate(&self, limits: &Limits) -> Result<(), ScenarioError> {
        if let Some(kind) = &self.kind
            && kind != "http_scenario"
        {
            return Err(ScenarioError::Kind);
        }
        if self.name.is_empty() {
            return Err(ScenarioError::Empty { field: "name" });
        }
        if self.config.is_empty() {
            return Err(ScenarioError::Empty { field: "config" });
        }
        validate_env(&self.env)?;
        if self.steps.len() < limits.min_steps || self.steps.len() > limits.max_steps {
            return Err(ScenarioError::StepCount {
                min: limits.min_steps,
                max: limits.max_steps,
            });
        }
        validate_steps(&self.steps)?;
        if self.setup.row_count() > limits.max_rows {
            return Err(ScenarioError::TooManyRows {
                max: limits.max_rows,
            });
        }
        if self.setup.fixture_bytes() > limits.max_fixture_bytes {
            return Err(ScenarioError::FixturesTooLarge {
                limit: limits.max_fixture_bytes,
            });
        }
        Ok(())
    }
}

fn validate_env(env: &BTreeMap<String, String>) -> Result<(), ScenarioError> {
    env.keys()
        .find(|name| RESERVED_ENV.contains(&name.as_str()))
        .map_or(Ok(()), |name| {
            Err(ScenarioError::ReservedEnv { name: name.clone() })
        })
}

fn validate_steps(steps: &[Step]) -> Result<(), ScenarioError> {
    let secrets = collect_captures(steps)?;
    for (index, step) in steps.iter().enumerate() {
        validate_step(index, step)?;
    }
    reject_secret_headers(steps, &secrets)
}

fn collect_captures(steps: &[Step]) -> Result<BTreeMap<String, bool>, ScenarioError> {
    let mut secrets = BTreeMap::new();
    for (index, step) in steps.iter().enumerate() {
        for capture in &step.capture {
            if capture.name.is_empty() {
                return Err(ScenarioError::StepField {
                    index,
                    field: "capture name",
                });
            }
            if secrets
                .insert(capture.name.clone(), capture.secret)
                .is_some()
            {
                return Err(ScenarioError::DuplicateCapture {
                    index,
                    name: capture.name.clone(),
                });
            }
        }
    }
    Ok(secrets)
}

fn reject_secret_headers(
    steps: &[Step],
    secrets: &BTreeMap<String, bool>,
) -> Result<(), ScenarioError> {
    steps.iter().enumerate().try_for_each(|(index, step)| {
        step.request
            .headers
            .iter()
            .try_for_each(|(header, value)| reject_header(index, header, value, secrets))
    })
}

fn reject_header(
    index: usize,
    header: &str,
    value: &str,
    secrets: &BTreeMap<String, bool>,
) -> Result<(), ScenarioError> {
    let secret = segments(value, index)?.into_iter().find_map(|segment| {
        let Segment::Capture(name) = segment else {
            return None;
        };
        (secrets.get(name) == Some(&true)).then(|| name.to_owned())
    });
    secret.map_or(Ok(()), |name| {
        Err(ScenarioError::SecretCaptureHeader {
            index,
            header: header.to_owned(),
            name,
        })
    })
}

fn validate_step(index: usize, step: &Step) -> Result<(), ScenarioError> {
    let empty = [
        ("name", step.name.is_empty()),
        ("request.path", step.request.path.is_empty()),
    ];
    if let Some((field, _)) = empty.iter().find(|(_, missing)| *missing) {
        return Err(ScenarioError::StepField { index, field });
    }
    request_body(&step.request.body, step.request.body_kind)
        .map_err(|source| ScenarioError::Body { index, source })?;
    segments(&step.request.path, index)?;
    for value in step.request.headers.values() {
        segments(value, index)?;
    }
    validate_ignores(index, &step.compare.ignore)
}

fn validate_ignores(index: usize, ignores: &[Ignore]) -> Result<(), ScenarioError> {
    ignores.iter().try_for_each(|ignored| {
        let problem = if !ignored.pointer.is_empty() && !ignored.pointer.starts_with('/') {
            "pointers must be JSON pointers"
        } else if ignored.why.is_empty() {
            "entries need a why"
        } else {
            return Ok(());
        };
        Err(ScenarioError::Ignore { index, problem })
    })
}
