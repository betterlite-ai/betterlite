use std::collections::BTreeMap;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

use crate::case::{Case, CaseError};
use crate::cli::Options;
use crate::compare;
use crate::http::{self, Client, HttpError, Response};
use crate::mock::{MockError, MockProvider};
use crate::process::{Gateway, ProcessError};

use super::gateway::{GatewayInput, start as start_gateway};

#[path = "artifacts.rs"]
pub(crate) mod artifacts;

#[derive(Debug, Error)]
pub(crate) enum HarnessError {
    #[error("{0}")]
    Cli(#[from] clap::Error),
    #[error("{0}")]
    Cases(#[from] CaseError),
    #[error("{0}")]
    Http(#[from] HttpError),
    #[error("{0}")]
    Mock(#[from] MockError),
    #[error("{0}")]
    Process(#[from] ProcessError),
    #[error("filesystem error at {path}: {source}")]
    Filesystem {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot serialize artifact: {0}")]
    Json(#[source] serde_json::Error),
    #[error("system clock error: {0}")]
    Clock(#[source] std::time::SystemTimeError),
}

pub(crate) struct Captured {
    pub(crate) response: Response,
}

pub(crate) fn run_cases(
    options: &Options,
    cases: Vec<(PathBuf, Case)>,
) -> Result<ExitCode, HarnessError> {
    if cases.is_empty() {
        return Err(missing_capture(0));
    }
    let mut groups = BTreeMap::<String, Vec<(PathBuf, Case)>>::new();
    for (path, case) in cases {
        let group_key = format!("{}\n{:?}", case.config, case.env);
        groups.entry(group_key).or_default().push((path, case));
    }
    let root = artifact_root()?;
    let mut different = false;
    for (_, group) in groups {
        different = accumulate_difference(different, run_group(options, &root, group)?);
    }
    if different {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

pub(crate) fn run_group(
    options: &Options,
    root: &Path,
    cases: Vec<(PathBuf, Case)>,
) -> Result<bool, HarnessError> {
    let upstream_answers = cases
        .iter()
        .flat_map(|(_, case)| case.upstream.iter())
        .cloned()
        .collect::<Vec<_>>();
    let mut answers = upstream_answers.clone();
    answers.extend(upstream_answers.iter().cloned());
    let mock = MockProvider::start(&answers)?;
    let group_dir = root.join(format!("group-{}", unique_id()?));
    create_dir(&group_dir)?;
    let config_path = group_dir.join("config.yaml");
    let case_env = cases
        .first()
        .map(|(_, case)| &case.env)
        .ok_or_else(|| missing_capture(0))?;
    let config = cases
        .first()
        .map(|(_, case)| case.config.replace("{{upstream}}", &mock.base_url))
        .unwrap_or_default();
    write_text(&config_path, &config)?;
    let client = Client;
    let first = start_gateway(GatewayInput {
        options,
        group_dir: &group_dir,
        config_path: &config_path,
        upstream: &mock.base_url,
        case_env,
        first: true,
    })?;
    let first_log = first.log_path.to_owned();
    let first_results = capture_gateway(&client, first, &cases)?;
    let second = start_gateway(GatewayInput {
        options,
        group_dir: &group_dir,
        config_path: &config_path,
        upstream: &mock.base_url,
        case_env,
        first: false,
    })?;
    let second_log = second.log_path.to_owned();
    let second_results = capture_gateway(&client, second, &cases)?;
    let provider_requests = mock.finish()?;
    let midpoint = upstream_answers.len();
    let first_requests = provider_requests.get(..midpoint).unwrap_or_default();
    let second_requests = provider_requests.get(midpoint..).unwrap_or_default();
    let mut different = false;
    let mut first_offset = 0;
    let mut second_offset = 0;
    for (index, (_, case)) in cases.iter().enumerate() {
        let request_count = case.upstream.len();
        let first_capture = first_results
            .get(index)
            .ok_or_else(|| missing_capture(index))?;
        let second_capture = second_results
            .get(index)
            .ok_or_else(|| missing_capture(index))?;
        let first_slice = request_slice(first_requests, first_offset, request_count);
        let second_slice = request_slice(second_requests, second_offset, request_count);
        first_offset = advance_offset(first_offset, request_count);
        second_offset = advance_offset(second_offset, request_count);
        let result = compare::compare(
            &first_capture.response,
            &second_capture.response,
            first_slice,
            second_slice,
            &case.compare,
        );
        let difference = result.as_ref().err();
        artifacts::write(artifacts::ArtifactInput {
            root,
            case,
            first: first_capture,
            second: second_capture,
            first_requests: first_slice,
            second_requests: second_slice,
            first_log: &first_log,
            second_log: &second_log,
            result: difference,
        })?;
        match result {
            Ok(()) => println!("PASS {}", case.name),
            Err(error) => {
                different = true;
                println!("FAIL {}: {}", case.name, error.pointer);
            }
        }
    }
    Ok(different)
}

pub(crate) fn request_slice(
    requests: &[crate::mock::ProviderRequest],
    offset: usize,
    count: usize,
) -> &[crate::mock::ProviderRequest] {
    let end = offset.saturating_add(count);
    requests.get(offset..end).unwrap_or_default()
}

pub(crate) fn advance_offset(offset: usize, count: usize) -> usize {
    let next = offset.saturating_add(count);
    next
}

pub(crate) fn accumulate_difference(first: bool, second: bool) -> bool {
    let different = first || second;
    different
}

pub(crate) fn capture_gateway(
    client: &Client,
    mut gateway: Gateway,
    cases: &[(PathBuf, Case)],
) -> Result<Vec<Captured>, HarnessError> {
    let mut captures = Vec::with_capacity(cases.len());
    for (_, case) in cases {
        let response = http::send(client, &gateway.base_url, &case.request)?;
        captures.push(Captured { response });
    }
    gateway.stop()?;
    Ok(captures)
}

pub(crate) fn artifact_root() -> Result<PathBuf, HarnessError> {
    let root = std::env::current_dir().map_err(|source| HarnessError::Filesystem {
        path: PathBuf::from("."),
        source,
    })?;
    let path = root.join("target/oracle");
    create_dir(&path)?;
    Ok(path)
}

pub(crate) fn create_dir(path: &Path) -> Result<(), HarnessError> {
    let result = fs::create_dir_all(path).map_err(|source| HarnessError::Filesystem {
        path: path.to_owned(),
        source,
    });
    result
}

pub(crate) fn write_text(path: &Path, text: &str) -> Result<(), HarnessError> {
    let result = fs::write(path, text).map_err(|source| HarnessError::Filesystem {
        path: path.to_owned(),
        source,
    });
    result
}

pub(crate) fn free_port() -> Result<u16, HarnessError> {
    let listener =
        TcpListener::bind(("127.0.0.1", 0)).map_err(|source| HarnessError::Filesystem {
            path: PathBuf::from("127.0.0.1:0"),
            source,
        })?;
    let port = listener
        .local_addr()
        .map_err(|source| HarnessError::Filesystem {
            path: PathBuf::from("127.0.0.1:0"),
            source,
        })?;
    Ok(port.port())
}

pub(crate) fn unique_id() -> Result<u128, HarnessError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(HarnessError::Clock)?;
    let nanos = elapsed.as_nanos();
    let pid = u128::from(std::process::id());
    Ok(combine_id(nanos, pid))
}

pub(crate) fn combine_id(nanos: u128, pid: u128) -> u128 {
    let combined = nanos ^ pid;
    combined
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

fn missing_capture(index: usize) -> HarnessError {
    let error = HarnessError::Filesystem {
        path: PathBuf::from(format!("case-{index}")),
        source: std::io::Error::other("gateway did not return a response"),
    };
    error
}
