use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use betterlite_bedrock::agentcore_stream::IdSource;
use betterlite_bedrock::realtime_session::update::SessionUpdate;
use serde::Deserialize;
use uuid::Uuid;

use crate::helpers::{Helper, HelperResult};
use wire::parse_ids;

mod cases;
mod tests;
mod wire;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    case: Case,
    ids: Vec<String>,
    #[serde(default)]
    model: String,
    #[serde(default)]
    trace_id: String,
    #[serde(default)]
    updates: Vec<SessionUpdate>,
    #[serde(default)]
    urls: Vec<Option<String>>,
    #[serde(default)]
    modalities: Option<Vec<String>>,
    #[serde(default)]
    models: Vec<String>,
    #[serde(default)]
    headers: Vec<Vec<String>>,
    #[serde(default)]
    error: Option<ErrorRun>,
}

#[derive(Debug, Deserialize)]
enum Case {
    #[serde(rename = "default_session_configuration")]
    DefaultSessionConfiguration,
    #[serde(rename = "session_update_options")]
    SessionUpdateOptions,
    #[serde(rename = "pcm16_input_output_rates")]
    Pcm16InputOutputRates,
    #[serde(rename = "g711_rates")]
    G711Rates,
    #[serde(rename = "explicit_sample_rate")]
    ExplicitSampleRate,
    #[serde(rename = "tool_configuration")]
    ToolConfiguration,
    #[serde(rename = "system_instructions")]
    SystemInstructions,
    #[serde(rename = "created_updated_events")]
    CreatedUpdatedEvents,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorRun {
    message: String,
    status: u16,
    headers: Vec<Vec<String>>,
}

pub(crate) struct Op;

pub(crate) struct FixedIds {
    values: Mutex<VecDeque<Uuid>>,
}

impl FixedIds {
    pub(crate) fn new(values: &[Uuid]) -> Self {
        Self {
            values: Mutex::new(values.iter().copied().collect()),
        }
    }
}

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values.pop_front().unwrap_or_else(Uuid::nil)
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let ids = parse_ids(&input.ids)?;
        match input.case {
            Case::DefaultSessionConfiguration => cases::default_case(&ids),
            Case::CreatedUpdatedEvents => cases::created_updated(input),
            Case::ToolConfiguration => cases::tool_values(input, &ids),
            Case::SessionUpdateOptions
            | Case::Pcm16InputOutputRates
            | Case::G711Rates
            | Case::ExplicitSampleRate
            | Case::SystemInstructions => cases::update_values(input, &ids),
        }
    }
}
