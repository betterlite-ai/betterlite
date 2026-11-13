use std::future::Future;

use betterlite_lens::models::execution::{Execution, ExecutionContent, Sample};
use betterlite_lens::models::finding::Evidence;
use betterlite_lens::models::settings::{LensSettings, MetadataFilter, Scope};
use betterlite_lens::sources::activity_availability::ActivityAvailability;
use betterlite_lens::sources::agent_row::AgentRow;
use betterlite_lens::sources::count_row::CountRow;
use betterlite_lens::sources::execution_id::execution_id;
use betterlite_lens::sources::execution_row::ExecutionRow;
use betterlite_lens::sources::parameters::parameters;
use betterlite_lens::sources::parse_execution::parse_execution;
use betterlite_lens::sources::part_row::PartRow;
use betterlite_lens::sources::selection_id::selection_id;
use betterlite_lens::sources::source_reader::SourceReader;
use betterlite_lens::sources::source_reader_sample::SampleWindow;

use self::fake_storage::FakeStorage;
use crate::helpers::{HelperError, HelperResult};

mod fake_storage;

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WindowInput {
    start: i64,
    end: i64,
    #[serde(default)]
    offset: i64,
    page_size: usize,
    #[serde(default)]
    preview: bool,
    #[serde(default)]
    cursor: String,
}

fn to_window(input: &WindowInput) -> SampleWindow {
    let cursor = input.cursor.clone();
    SampleWindow {
        start: input.start,
        end: input.end,
        offset: input.offset,
        page_size: input.page_size,
        preview: input.preview,
        cursor,
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ExecutionId {
        source: String,
        team_id: String,
        trace_id: String,
        trace_ref: String,
    },
    ParseExecution {
        value: String,
    },
    SelectionId {
        value: String,
    },
    Parameters {
        scope: Scope,
        filters: Vec<MetadataFilter>,
    },
    Availability {
        scope: Scope,
        rows: Vec<ActivityAvailability>,
    },
    Agents {
        scope: Scope,
        rows: Vec<AgentRow>,
    },
    Sample {
        scope: Scope,
        settings: LensSettings,
        window: WindowInput,
        rows: Vec<ExecutionRow>,
    },
    Content {
        scope: Scope,
        execution: Execution,
        cursor: String,
        offset: i64,
        rows: Vec<PartRow>,
    },
    VerifyEvidence {
        scope: Scope,
        execution: Execution,
        evidence: Evidence,
        rows: Vec<CountRow>,
    },
}

fn failure(class: &str, message: String) -> HelperResult {
    let error = HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    };
    Err(Box::new(error))
}

fn source_error<E>(error: E) -> Box<HelperError>
where
    E: std::fmt::Display,
{
    let message = error.to_string();
    Box::new(HelperError {
        class: "SourceError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn runtime<F>(future: F) -> HelperResult
where
    F: Future<Output = HelperResult>,
{
    let built = tokio::runtime::Builder::new_current_thread().build();
    match built {
        Ok(rt) => rt.block_on(future),
        Err(error) => failure("RuntimeError", error.to_string()),
    }
}

fn fake_reader<T>(rows: &[T]) -> Result<SourceReader<FakeStorage>, Box<HelperError>>
where
    T: serde::Serialize,
{
    let bytes = serde_json::to_vec(rows).map_err(source_error)?;
    Ok(SourceReader {
        storage: FakeStorage(bytes),
    })
}

async fn availability(scope: &Scope, rows: &[ActivityAvailability]) -> HelperResult {
    let result = fake_reader(rows)?
        .availability(scope)
        .await
        .map_err(source_error)?;
    serde_json::to_value(result).or_else(|error| failure("TypeError", error.to_string()))
}

async fn agents(scope: &Scope, rows: &[AgentRow]) -> HelperResult {
    let result = fake_reader(rows)?
        .agents(scope)
        .await
        .map_err(source_error)?;
    serde_json::to_value(result).or_else(|error| failure("TypeError", error.to_string()))
}

async fn sample(
    scope: &Scope,
    settings: &LensSettings,
    window: SampleWindow,
    rows: &[ExecutionRow],
) -> HelperResult {
    let result: Sample = fake_reader(rows)?
        .sample(scope, settings, window)
        .await
        .map_err(source_error)?;
    serde_json::to_value(result).or_else(|error| failure("TypeError", error.to_string()))
}

async fn content(
    scope: &Scope,
    execution: &Execution,
    cursor: &str,
    offset: i64,
    rows: &[PartRow],
) -> HelperResult {
    let result: ExecutionContent = fake_reader(rows)?
        .content(scope, execution, cursor, offset)
        .await
        .map_err(source_error)?;
    serde_json::to_value(result).or_else(|error| failure("TypeError", error.to_string()))
}

async fn verify_evidence(
    scope: &Scope,
    execution: &Execution,
    evidence: &Evidence,
    rows: &[CountRow],
) -> HelperResult {
    let result = fake_reader(rows)?
        .verify_evidence(scope, execution, evidence)
        .await
        .map_err(source_error)?;
    serde_json::to_value(result).or_else(|error| failure("TypeError", error.to_string()))
}

fn run(input: &Input) -> HelperResult {
    let serialized = match input {
        Input::ExecutionId {
            source,
            team_id,
            trace_id,
            trace_ref,
        } => serde_json::to_value(execution_id(source, team_id, trace_id, trace_ref)),
        Input::ParseExecution { value: id } => match parse_execution(id) {
            Ok(parsed) => serde_json::to_value(parsed),
            Err(error) => return failure("ParseExecutionError", error.to_string()),
        },
        Input::SelectionId { value: id } => match selection_id(id) {
            Ok(selected) => serde_json::to_value(selected),
            Err(error) => return failure("ParseExecutionError", error.to_string()),
        },
        Input::Parameters { scope, filters } => serde_json::to_value(parameters(scope, filters)),
        Input::Availability { scope, rows } => return runtime(availability(scope, rows)),
        Input::Agents { scope, rows } => return runtime(agents(scope, rows)),
        Input::Sample {
            scope,
            settings,
            window,
            rows,
        } => return runtime(sample(scope, settings, to_window(window), rows)),
        Input::Content {
            scope,
            execution,
            cursor,
            offset,
            rows,
        } => return runtime(content(scope, execution, cursor, *offset, rows)),
        Input::VerifyEvidence {
            scope,
            execution,
            evidence,
            rows,
        } => return runtime(verify_evidence(scope, execution, evidence, rows)),
    };
    serialized.or_else(|error| failure("TypeError", error.to_string()))
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        run(input)
    }
}
