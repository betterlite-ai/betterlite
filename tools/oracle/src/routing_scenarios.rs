use std::path::Path;

use serde::Deserialize;

use crate::OracleError;
use crate::case::{Compare, Ignore, Request};
use crate::http;
use crate::multipart_oracle::request::request_body;
use crate::routing_mock_scripts::Wait;

pub(crate) mod compare;
pub(crate) mod run;
mod tests;

pub(crate) const MAX_STEPS: usize = 256;
pub(crate) const MAX_CONCURRENT: usize = 32;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub(crate) steps: Vec<ScenarioStep>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ScenarioStep {
    Http(HttpStep),
    Concurrent(ConcurrentStep),
    Join(JoinPoint),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HttpStep {
    pub(crate) request: Request,
    #[serde(default)]
    pub(crate) compare: Compare,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConcurrentStep {
    pub(crate) requests: Vec<HttpStep>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JoinPoint {
    #[serde(default)]
    pub(crate) deadline_ms: Option<u64>,
}

pub fn parse_scenario(bytes: &[u8]) -> Result<Scenario, OracleError> {
    if bytes.len() > http::MAX_BODY {
        return Err(OracleError::ScenarioTooLarge { bytes: bytes.len() });
    }
    let scenario = serde_json::from_slice::<Scenario>(bytes).map_err(OracleError::ScenarioParse)?;
    validate(&scenario).map_err(|message| OracleError::ScenarioInvalid { message })?;
    Ok(scenario)
}

pub(crate) fn validate_in_case(path: &Path, scenario: &Scenario) -> Result<(), OracleError> {
    validate(scenario).map_err(|message| OracleError::CaseInvalid {
        path: path.to_owned(),
        message,
    })
}

// Only a concurrent step's parties share a barrier, one per step and keyed by case.
pub(crate) fn upstream_waits(case_name: &str, scenario: &Scenario) -> Vec<Wait> {
    scenario
        .steps
        .iter()
        .enumerate()
        .flat_map(|(index, step)| match step {
            ScenarioStep::Http(_) => vec![Wait::None],
            ScenarioStep::Concurrent(concurrent) => {
                let name = format!("{case_name}-concurrent-{index}");
                vec![Wait::Barrier { name }; concurrent.requests.len()]
            }
            ScenarioStep::Join(_) => Vec::new(),
        })
        .collect()
}

fn validate(scenario: &Scenario) -> Result<(), String> {
    if scenario.steps.is_empty() {
        return Err("scenario.steps cannot be empty".to_owned());
    }
    if scenario.steps.len() > MAX_STEPS {
        return Err(format!("scenario.steps cannot exceed {MAX_STEPS}"));
    }
    let mut pending = false;
    for step in &scenario.steps {
        match step {
            ScenarioStep::Http(http_step) => validate_http_step(http_step)?,
            ScenarioStep::Concurrent(concurrent) => {
                if pending {
                    return Err(
                        "a concurrent step cannot start before the previous one joins".to_owned(),
                    );
                }
                validate_concurrent_step(concurrent)?;
                pending = true;
            }
            ScenarioStep::Join(join) => {
                if !pending {
                    return Err("join has no concurrent step to join".to_owned());
                }
                validate_join_point(join)?;
                pending = false;
            }
        }
    }
    if pending {
        return Err("scenario ends with an unjoined concurrent step".to_owned());
    }
    Ok(())
}

fn validate_http_step(step: &HttpStep) -> Result<(), String> {
    if step.request.path.is_empty() {
        return Err("scenario request.path cannot be empty".to_owned());
    }
    request_body(&step.request.body, step.request.body_kind)
        .map_err(|source| source.to_string())?;
    ignores_are_valid(&step.compare.ignore)
}

fn validate_concurrent_step(step: &ConcurrentStep) -> Result<(), String> {
    if step.requests.len() < 2 {
        return Err("concurrent.requests needs at least two requests".to_owned());
    }
    if step.requests.len() > MAX_CONCURRENT {
        return Err(format!(
            "concurrent.requests cannot exceed {MAX_CONCURRENT}"
        ));
    }
    step.requests.iter().try_for_each(validate_http_step)
}

fn validate_join_point(join: &JoinPoint) -> Result<(), String> {
    match join.deadline_ms {
        None => Ok(()),
        Some(millis) if (1..=http::REQUEST_TIMEOUT_MS).contains(&millis) => Ok(()),
        Some(millis) => Err(format!(
            "join.deadline_ms {millis} is outside 1 to {}",
            http::REQUEST_TIMEOUT_MS
        )),
    }
}

fn ignores_are_valid(ignores: &[Ignore]) -> Result<(), String> {
    for ignored in ignores {
        if !ignored.pointer.is_empty() && !ignored.pointer.starts_with('/') {
            return Err("compare.ignore pointers must be JSON pointers".to_owned());
        }
        if ignored.why.is_empty() {
            return Err("compare.ignore entries need a why".to_owned());
        }
    }
    Ok(())
}
