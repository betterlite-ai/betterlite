use reqwest::blocking::Client;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, Write as _};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process;
use std::process::ExitCode;
use std::time::{SystemTime, SystemTimeError, UNIX_EPOCH};

use thiserror::Error;

use crate::case::Case;
use crate::cli::Options;
use crate::compare;
use crate::http::{self, HttpError, Response};
use crate::mock::{MockError, MockProvider, ProviderRequest};
use crate::process::{Gateway, ProcessError};

use super::gateway::{GatewayInput, start as start_gateway};

#[path = "artifacts.rs"]
pub(crate) mod artifacts;

#[derive(Debug, Error)]
pub enum HarnessError {
    #[error("gateway did not return a response for case {0}")]
    MissingCapture(usize),
    #[error("cannot write progress: {0}")]
    Output(#[source] io::Error),
    #[error("{0}")]
    Http(#[from] HttpError),
    #[error("{0}")]
    Mock(#[from] MockError),
    #[error("{0}")]
    Process(#[from] ProcessError),
    #[error("filesystem error at {path}: {source}")]
    Filesystem { path: PathBuf, source: io::Error },
    #[error("cannot serialize artifact: {0}")]
    Json(#[source] serde_json::Error),
    #[error("system clock error: {0}")]
    Clock(#[source] SystemTimeError),
}

pub(crate) fn run_cases(options: &Options, cases: Vec<Case>) -> Result<ExitCode, HarnessError> {
    if cases.is_empty() {
        return Err(HarnessError::MissingCapture(0));
    }
    let mut groups = BTreeMap::<String, Vec<Case>>::new();
    for case in cases {
        let group_key = format!("{}\n{:?}", case.config, case.env);
        groups.entry(group_key).or_default().push(case);
    }
    let root = artifact_root()?;
    let mut outcomes = Vec::new();
    for (_, group) in groups {
        outcomes.extend(run_group(options, &root, &group)?);
    }
    Ok(exit_status(outcomes.contains(&true)))
}

pub(crate) fn exit_status(different: bool) -> ExitCode {
    if different {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

pub(crate) fn run_group(
    options: &Options,
    root: &Path,
    cases: &[Case],
) -> Result<Vec<bool>, HarnessError> {
    let upstream_answers = cases
        .iter()
        .flat_map(|case| case.upstream.iter())
        .cloned()
        .collect::<Vec<_>>();
    let mock = MockProvider::start(&[upstream_answers.clone(), upstream_answers.clone()].concat())?;
    let first_case = cases.first().ok_or(HarnessError::MissingCapture(0))?;
    let (group_dir, config_path) = artifacts::prepare(root, first_case, &mock.base_url)?;
    let client = http::client()?;
    let input = GatewayInput {
        options,
        group_dir: &group_dir,
        config_path: &config_path,
        case_env: &first_case.env,
    };
    let first = start_gateway(&input, true)?;
    let first_log = first.log_path.clone();
    let first_results = capture_gateway(&client, first, cases)?;
    let second = start_gateway(&input, false)?;
    let second_log = second.log_path.clone();
    let second_results = capture_gateway(&client, second, cases)?;
    let captures = GroupCapture {
        first: first_results,
        second: second_results,
        first_log,
        second_log,
        requests: mock.finish()?,
        midpoint: upstream_answers.len(),
    };
    compare_group(root, cases, &captures)
}

pub(crate) struct GroupCapture {
    pub(crate) first: Vec<Response>,
    pub(crate) second: Vec<Response>,
    pub(crate) first_log: PathBuf,
    pub(crate) second_log: PathBuf,
    pub(crate) requests: Vec<ProviderRequest>,
    pub(crate) midpoint: usize,
}

pub(crate) fn compare_group(
    root: &Path,
    cases: &[Case],
    captures: &GroupCapture,
) -> Result<Vec<bool>, HarnessError> {
    let (first_requests, second_requests) = captures
        .requests
        .split_at_checked(captures.midpoint)
        .unwrap_or_default();
    let mut outcomes = Vec::with_capacity(cases.len());
    let mut first_offset = 0;
    for (index, case) in cases.iter().enumerate() {
        let request_count = case.upstream.len();
        let first_capture = captures
            .first
            .get(index)
            .ok_or(HarnessError::MissingCapture(index))?;
        let second_capture = captures
            .second
            .get(index)
            .ok_or(HarnessError::MissingCapture(index))?;
        let first_slice = request_slice(first_requests, first_offset, request_count);
        let second_slice = request_slice(second_requests, first_offset, request_count);
        first_offset = first_offset.saturating_add(request_count);
        outcomes.push(compare_case(artifacts::ArtifactInput {
            root,
            case,
            first: first_capture,
            second: second_capture,
            first_requests: first_slice,
            second_requests: second_slice,
            first_log: &captures.first_log,
            second_log: &captures.second_log,
            result: None,
        })?);
    }
    Ok(outcomes)
}

fn compare_case(input: artifacts::ArtifactInput<'_>) -> Result<bool, HarnessError> {
    let result = compare::compare(
        input.first,
        input.second,
        input.first_requests,
        input.second_requests,
        &input.case.compare,
    );
    artifacts::write(artifacts::ArtifactInput {
        result: result.as_ref().err(),
        ..input
    })?;
    let name = &input.case.name;
    match result {
        Ok(()) => {
            writeln!(io::stdout().lock(), "PASS {name}").map_err(HarnessError::Output)?;
            Ok(false)
        }
        Err(error) => {
            writeln!(io::stdout().lock(), "FAIL {name}: {}", error.pointer)
                .map_err(HarnessError::Output)?;
            Ok(true)
        }
    }
}

pub(crate) fn request_slice(
    requests: &[ProviderRequest],
    offset: usize,
    count: usize,
) -> &[ProviderRequest] {
    let end = offset.saturating_add(count);
    requests.get(offset..end).unwrap_or_default()
}

pub(crate) fn capture_gateway(
    client: &Client,
    mut gateway: Gateway,
    cases: &[Case],
) -> Result<Vec<Response>, HarnessError> {
    let mut captures = Vec::with_capacity(cases.len());
    for case in cases {
        let response = http::send(client, &gateway.base_url, &case.request)?;
        captures.push(response);
    }
    gateway.stop()?;
    Ok(captures)
}

pub(crate) fn artifact_root() -> Result<PathBuf, HarnessError> {
    let root = env::current_dir().map_err(|source| HarnessError::Filesystem {
        path: PathBuf::from("."),
        source,
    })?;
    let path = root.join("target/oracle");
    fs::create_dir_all(&path).map_err(|source| HarnessError::Filesystem {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

pub(crate) fn free_port() -> Result<u16, HarnessError> {
    let port = TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| listener.local_addr())
        .map_err(|source| HarnessError::Filesystem {
            path: PathBuf::from("127.0.0.1:0"),
            source,
        })?;
    Ok(port.port())
}

pub(crate) fn unique_id() -> Result<String, HarnessError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(HarnessError::Clock)?;
    Ok(format!("{}-{}", process::id(), elapsed.as_nanos()))
}

pub(crate) fn safe_name(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "case".to_owned()
    } else {
        sanitized
    }
}
