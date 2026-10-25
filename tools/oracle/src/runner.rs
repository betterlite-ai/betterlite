use reqwest::blocking::Client;
use std::collections::BTreeMap;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::OracleError;
use crate::artifacts;
use crate::case::{Case, LoadedCase};
use crate::cli::Options;
use crate::compare;
use crate::helpers;
use crate::http::{self, Response};
use crate::mock::{MockProvider, ProviderRequest};
use crate::paths::artifact_root;
use crate::process::Gateway;
use crate::routing_mock_scripts::MockScript;
use betterlite_postgres_fixture::connection::PostgresFixture;

use super::gateway::{GatewayInput, start as start_gateway};

pub(crate) fn run_cases(
    options: &Options,
    cases: Vec<LoadedCase>,
) -> Result<ExitCode, OracleError> {
    if cases.is_empty() {
        return Err(OracleError::MissingCapture(0));
    }
    let mut groups = BTreeMap::<String, Vec<Case>>::new();
    let mut helper_outcome = helpers::HelperRun::Match;
    for loaded_case in cases {
        match loaded_case {
            LoadedCase::Http(http_case) => {
                let group_key = format!("{}\n{:?}", http_case.config, http_case.env);
                groups.entry(group_key).or_default().push(http_case);
            }
            LoadedCase::Helper(helper_case) => {
                let result = helpers::run_case(&helper_case, options.self_check)?;
                if matches!(result, helpers::HelperRun::Difference) {
                    helper_outcome = helpers::HelperRun::Difference;
                }
            }
        }
    }
    let root = artifact_root()?;
    let mut outcomes = Vec::new();
    for (_, group) in groups {
        outcomes.extend(run_group(options, &root, &group)?);
    }
    if matches!(helper_outcome, helpers::HelperRun::Difference) {
        return Ok(exit_status(true));
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
) -> Result<Vec<bool>, OracleError> {
    let upstream_answers = cases
        .iter()
        .flat_map(|case| case.upstream.iter())
        .cloned()
        .collect::<Vec<_>>();
    let mock = MockProvider::start(MockScript::try_from(upstream_answers.as_slice())?)?;
    let first_case = cases.first().ok_or(OracleError::MissingCapture(0))?;
    let (first_fixture, second_fixture) = provision_fixtures(cases)?;
    let (group_dir, config_path) = artifacts::prepare(root, first_case, &mock.base_url)?;
    let client = http::client()?;
    let input = GatewayInput {
        options,
        group_dir: &group_dir,
        config_path: &config_path,
        case_env: &first_case.env,
    };
    let captures = capture_gateways(&input, cases, &client, mock)?;
    let result = compare_group(root, cases, &captures);
    close_fixture(first_fixture)?;
    close_fixture(second_fixture)?;
    result
}

fn capture_gateways(
    input: &GatewayInput<'_>,
    cases: &[Case],
    client: &Client,
    mock: MockProvider,
) -> Result<GroupCapture, OracleError> {
    let first = start_gateway(input, true)?;
    let first_log = first.log_path.clone();
    let first_results = capture_gateway(client, first, cases)?;
    let first_trace = mock.finish_run()?;
    let second = start_gateway(input, false)?;
    let second_log = second.log_path.clone();
    let second_results = capture_gateway(client, second, cases)?;
    let second_requests = mock
        .finish()?
        .into_iter()
        .map(|call| call.request)
        .collect();
    Ok(GroupCapture {
        first: first_results,
        second: second_results,
        first_log,
        second_log,
        first_requests: first_trace.into_iter().map(|call| call.request).collect(),
        second_requests,
    })
}

fn provision_fixtures(
    cases: &[Case],
) -> Result<(Option<PostgresFixture>, Option<PostgresFixture>), OracleError> {
    let database_fixture = cases.iter().find_map(|case| case.database.clone());
    let Some(database) = database_fixture else {
        return Ok((None, None));
    };
    let configured_url = cases
        .iter()
        .find_map(|case| case.env.get("POSTGRES_TEST_URL").map(String::as_str));
    let first = PostgresFixture::provision_from_url(configured_url, &database)
        .map_err(OracleError::DatabaseFixture)?;
    let second = match PostgresFixture::provision_from_url(configured_url, &database) {
        Ok(second_database) => second_database,
        Err(error) => {
            drop(first.close());
            return Err(OracleError::DatabaseFixture(error));
        }
    };
    Ok((Some(first), Some(second)))
}

fn close_fixture(maybe_database: Option<PostgresFixture>) -> Result<(), OracleError> {
    if let Some(created_database) = maybe_database {
        created_database
            .close()
            .map_err(OracleError::DatabaseFixture)?;
    }
    Ok(())
}

pub(crate) struct GroupCapture {
    pub(crate) first: Vec<Response>,
    pub(crate) second: Vec<Response>,
    pub(crate) first_log: PathBuf,
    pub(crate) second_log: PathBuf,
    pub(crate) first_requests: Vec<ProviderRequest>,
    pub(crate) second_requests: Vec<ProviderRequest>,
}

pub(crate) fn compare_group(
    root: &Path,
    cases: &[Case],
    captures: &GroupCapture,
) -> Result<Vec<bool>, OracleError> {
    let mut outcomes = Vec::with_capacity(cases.len());
    let mut first_offset = 0;
    for (index, case) in cases.iter().enumerate() {
        let request_count = case.upstream.len();
        let first_capture = captures
            .first
            .get(index)
            .ok_or(OracleError::MissingCapture(index))?;
        let second_capture = captures
            .second
            .get(index)
            .ok_or(OracleError::MissingCapture(index))?;
        let first_slice = request_slice(&captures.first_requests, first_offset, request_count);
        let second_slice = request_slice(&captures.second_requests, first_offset, request_count);
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

fn compare_case(input: artifacts::ArtifactInput<'_>) -> Result<bool, OracleError> {
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
            writeln!(io::stdout().lock(), "PASS {name}").map_err(OracleError::Output)?;
            Ok(false)
        }
        Err(error) => {
            writeln!(io::stdout().lock(), "FAIL {name}: {}", error.pointer)
                .map_err(OracleError::Output)?;
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
) -> Result<Vec<Response>, OracleError> {
    let mut captures = Vec::with_capacity(cases.len());
    for case in cases {
        let response = http::send(client, &gateway.base_url, &case.request)?;
        captures.push(response);
    }
    gateway.stop()?;
    Ok(captures)
}
