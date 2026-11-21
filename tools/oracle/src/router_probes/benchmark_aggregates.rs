use std::error::Error as StdError;

use betterlite_models::decisions::Json;
use betterlite_server::benchmark_aggregates::decode::{DecodeError, decode_session_agg_rows};
use betterlite_server::benchmark_aggregates::group::{Benchmarks, benchmarks};
use betterlite_server::benchmark_aggregates::row::SessionAggRow;
use betterlite_server::benchmark_aggregates::window::BenchmarkWindow;
use betterlite_server::routing::benchmark_aggregates::StrategyDeployment;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub now_unix_millis: i64,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub end_date: Option<String>,
    #[serde(default)]
    pub deployments: Vec<Deployment>,
    #[serde(default)]
    pub rows: Vec<Json>,
    #[serde(default)]
    pub parse: Option<ParsePayload>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deployment {
    #[serde(default)]
    pub model_name: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParsePayload {
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryBounds {
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ParseOutput {
    Valid { rows: usize },
    Invalid { bytes: usize, limit: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub response: Benchmarks,
    pub query: QueryBounds,
    #[serde(default)]
    pub parse: Option<ParseOutput>,
}

fn invalid(error: &dyn StdError) -> OperationError {
    OperationError::InvalidRequest {
        message: error.to_string(),
    }
}

fn invalid_message(message: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: message.to_owned(),
    }
}

fn window_of(input: &Input) -> Result<BenchmarkWindow, OperationError> {
    let today = DateTime::<Utc>::from_timestamp_millis(input.now_unix_millis)
        .map(|instant| instant.date_naive())
        .ok_or_else(|| invalid_message("the probe clock is outside the supported calendar"))?;
    BenchmarkWindow::resolve(
        input.start_date.as_deref(),
        input.end_date.as_deref(),
        today,
    )
    .map_err(|error| invalid(&error))
}

fn rows_of(input: &Input) -> Result<Vec<SessionAggRow>, OperationError> {
    let bytes = serde_json::to_vec(&input.rows).map_err(|error| invalid(&error))?;
    decode_session_agg_rows(&bytes).map_err(|error| invalid(&error))
}

fn parse_step(rows: &[Json], payload: &ParsePayload) -> Result<ParseOutput, OperationError> {
    let size = usize::try_from(payload.bytes).map_err(|error| invalid(&error))?;
    let mut document = serde_json::to_vec(rows).map_err(|error| invalid(&error))?;
    if document.len() > size {
        return Err(invalid_message(
            "the parse payload is shorter than the row document",
        ));
    }
    document.resize(size, b' ');
    match decode_session_agg_rows(&document) {
        Ok(decoded) => Ok(ParseOutput::Valid {
            rows: decoded.len(),
        }),
        Err(DecodeError::TooLarge { bytes, limit }) => Ok(ParseOutput::Invalid { bytes, limit }),
        Err(DecodeError::Invalid { source }) => Err(invalid(&source)),
    }
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let window = window_of(input)?;
    let rows = rows_of(input)?;
    let deployments: Vec<StrategyDeployment<'_>> = input
        .deployments
        .iter()
        .map(|deployment| StrategyDeployment {
            model_name: deployment.model_name.as_deref(),
            model: deployment.model.as_deref(),
        })
        .collect();
    let response = benchmarks(&window, &rows, Some(&deployments));
    let parse = input
        .parse
        .as_ref()
        .map(|payload| parse_step(&input.rows, payload))
        .transpose()?;
    Ok(ProbeOutput::BenchmarkAggregates(Output {
        response,
        query: QueryBounds {
            start: window.query_start.clone(),
            end: window.query_end.clone(),
        },
        parse,
    }))
}
