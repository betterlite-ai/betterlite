mod app;
mod driver;

use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::admission_control::Settings;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use tokio::runtime::Builder as RuntimeBuilder;

use crate::router_probes::ProbeOutput;
use crate::router_probes::ProbeOutput::AdmissionControl as AdmissionOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

use self::driver::drive;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    #[serde(default)]
    pub settings: BTreeMap<String, Value>,
    #[serde(default)]
    pub root_path: String,
    #[serde(default)]
    pub scalar_root: bool,
    pub requests: Vec<RequestSpec>,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestSpec {
    pub id: u16,
    pub path: String,
    pub behavior: Behavior,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Behavior {
    Immediate,
    Hold,
    Stream,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    Start { request: u16 },
    AwaitStarted { request: u16 },
    AwaitQueued { queued: u16 },
    Release { request: u16 },
    Cancel { request: u16 },
    Complete { request: u16 },
    Stats,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub limits: Option<LimitsOutput>,
    pub observations: Vec<Observation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitsOutput {
    pub max_in_flight_requests: u64,
    pub max_queued_requests: u64,
    pub queue_timeout_seconds: Number,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Observation {
    Started {
        request: u16,
    },
    Queued {
        queued: u64,
    },
    Stats {
        admitted: u64,
        queued: u64,
        rejected_total: u64,
        in_flight: u64,
    },
    Completed {
        request: u16,
        status: u16,
        retry_after: Option<String>,
        content_type: Option<String>,
        body: Option<Value>,
    },
    TimedOut {
        wait: String,
    },
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let ids = validate_requests(input)?;
    validate_steps(input, &ids)?;
    let settings = load_settings(&input.settings)?;
    let runtime = RuntimeBuilder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| OperationError::InvalidRequest {
            message: format!("the probe runtime could not start: {error}"),
        })?;
    runtime
        .block_on(drive(input, &settings))
        .map(AdmissionOutput)
}

fn load_settings(settings: &BTreeMap<String, Value>) -> Result<Settings, OperationError> {
    #[derive(Serialize)]
    struct ConfigView<'settings> {
        general_settings: &'settings BTreeMap<String, Value>,
    }
    let source = serde_saphyr::to_string(&ConfigView {
        general_settings: settings,
    })
    .map_err(|error| invalid(&format!("the probe settings are not valid YAML: {error}")))?;
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&source)
    .map_err(|error| {
        invalid(&format!(
            "the probe settings are not a valid config: {error}"
        ))
    })?;
    Ok(config.sections.admission_control)
}

fn invalid(message: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: message.to_owned(),
    }
}

fn validate_requests(input: &Input) -> Result<BTreeSet<u16>, OperationError> {
    let mut paths = BTreeSet::<&str>::new();
    let mut ids = BTreeSet::<u16>::new();
    for spec in &input.requests {
        if !paths.insert(spec.path.as_str()) {
            return Err(invalid("a request path is used twice"));
        }
        if !ids.insert(spec.id) {
            return Err(invalid("a request id is used twice"));
        }
    }
    Ok(ids)
}

fn validate_steps(input: &Input, ids: &BTreeSet<u16>) -> Result<(), OperationError> {
    let mut started = BTreeSet::<u16>::new();
    let mut finished = BTreeSet::<u16>::new();
    for step in &input.steps {
        match step {
            Step::Start { request } => {
                if !ids.contains(request) {
                    return Err(invalid("a step names an unknown request"));
                }
                if !started.insert(*request) {
                    return Err(invalid("a request is started twice"));
                }
            }
            Step::AwaitQueued { .. } | Step::Stats => {}
            Step::AwaitStarted { request }
            | Step::Release { request }
            | Step::Cancel { request }
            | Step::Complete { request } => {
                if !ids.contains(request) {
                    return Err(invalid("a step names an unknown request"));
                }
                if !started.contains(request) {
                    return Err(invalid("a step runs before its request starts"));
                }
                if matches!(step, Step::Cancel { .. } | Step::Complete { .. })
                    && !finished.insert(*request)
                {
                    return Err(invalid("a request terminates twice"));
                }
            }
        }
    }
    if started.len() != finished.len() {
        return Err(invalid("a started request never completes or cancels"));
    }
    Ok(())
}
