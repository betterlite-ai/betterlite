use std::error::Error as StdError;

use betterlite_models::decisions::Json;
use betterlite_server::benchmark_aggregates::decode::{DecodeError, decode_session_agg_rows};
use betterlite_server::benchmark_aggregates::group::{Benchmarks, benchmarks};
use betterlite_server::benchmark_aggregates::window::BenchmarkWindow;
use betterlite_server::routing::benchmark_aggregates::StrategyDeployment;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

// the enum in mod.rs would otherwise carry every field of this payload inline in its largest variant
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(Box<Payload>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    now_unix_millis: i64,
    #[serde(default)]
    start_date: Option<String>,
    #[serde(default)]
    end_date: Option<String>,
    #[serde(default)]
    deployments: Vec<Deployment>,
    #[serde(default)]
    rows: Vec<Json>,
    #[serde(default)]
    parse: Option<ParsePayload>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Deployment {
    #[serde(default)]
    model_name: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParsePayload {
    bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryBounds {
    start: String,
    end: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ParseOutput {
    Valid { rows: usize },
    Invalid { bytes: usize, limit: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Output(Box<Fields>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Fields {
    response: Benchmarks,
    query: QueryBounds,
    #[serde(default)]
    parse: Option<ParseOutput>,
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

fn window_of(payload: &Payload) -> Result<BenchmarkWindow, OperationError> {
    let today = DateTime::<Utc>::from_timestamp_millis(payload.now_unix_millis)
        .map(|instant| instant.date_naive())
        .ok_or_else(|| invalid_message("the probe clock is outside the supported calendar"))?;
    BenchmarkWindow::resolve(
        payload.start_date.as_deref(),
        payload.end_date.as_deref(),
        today,
    )
    .map_err(|error| invalid(&error))
}

fn parse_step(rows: &[Json], payload: ParsePayload) -> Result<ParseOutput, OperationError> {
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
    let payload = &input.0;
    let window = window_of(payload)?;
    let bytes = serde_json::to_vec(&payload.rows).map_err(|error| invalid(&error))?;
    let rows = decode_session_agg_rows(&bytes).map_err(|error| invalid(&error))?;
    let deployments: Vec<StrategyDeployment<'_>> = payload
        .deployments
        .iter()
        .map(|deployment| StrategyDeployment {
            model_name: deployment.model_name.as_deref(),
            model: deployment.model.as_deref(),
        })
        .collect();
    let response = benchmarks(&window, &rows, Some(&deployments));
    let parse = payload
        .parse
        .as_ref()
        .map(|step| parse_step(&payload.rows, *step))
        .transpose()?;
    Ok(ProbeOutput::BenchmarkAggregates(Output(Box::new(Fields {
        response,
        query: QueryBounds {
            start: window.query_start.clone(),
            end: window.query_end.clone(),
        },
        parse,
    }))))
}
