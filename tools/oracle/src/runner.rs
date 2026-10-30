use reqwest::blocking::Client;
use secrecy::SecretString;
use std::collections::BTreeMap;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use super::gateway::{GatewayInput, start as start_gateway};
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

mod config;
mod fixtures;
mod native_feature_eligibility;

#[derive(Debug, clap::Parser)]
pub(crate) struct DatabaseEnvironment {
    #[arg(long, hide = true, env = "POSTGRES_TEST_URL")]
    pub(crate) postgres_test_url: Option<SecretString>,
}

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
    let environment =
        <DatabaseEnvironment as clap::Parser>::try_parse_from([env!("CARGO_PKG_NAME")])?;
    run_group_with_url(options, root, cases, environment.postgres_test_url.as_ref())
}

pub(crate) fn run_group_with_url(
    options: &Options,
    root: &Path,
    cases: &[Case],
    configured_url: Option<&SecretString>,
) -> Result<Vec<bool>, OracleError> {
    if cases.is_empty() {
        return Err(OracleError::MissingCapture(0));
    }
    let upstream_answers = cases
        .iter()
        .flat_map(|case| case.upstream.iter())
        .cloned()
        .collect::<Vec<_>>();
    let mock = MockProvider::start(MockScript::try_from(upstream_answers.as_slice())?)?;
    let first_case = cases.first().ok_or(OracleError::MissingCapture(0))?;
    let fixtures = fixtures::provision(cases, configured_url)?;
    let operation_result = run_group_capture(GroupCaptureInput {
        options,
        root,
        cases,
        first_case,
        fixtures: &fixtures,
        mock,
    });
    let cleanup_result = fixtures.close();
    match (operation_result, cleanup_result) {
        (Ok(outcomes), Ok(())) => Ok(outcomes),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(cleanup_error)) => Err(OracleError::DatabaseCleanup(cleanup_error)),
        (Err(error), Err(cleanup_error)) => Err(OracleError::OperationCleanup {
            operation: Box::new(error),
            cleanup: cleanup_error,
        }),
    }
}

struct GroupCaptureInput<'input> {
    options: &'input Options,
    root: &'input Path,
    cases: &'input [Case],
    first_case: &'input Case,
    fixtures: &'input fixtures::FixtureGuard,
    mock: MockProvider,
}

fn run_group_capture(input: GroupCaptureInput<'_>) -> Result<Vec<bool>, OracleError> {
    let GroupCaptureInput {
        options,
        root,
        cases,
        first_case,
        fixtures,
        mock,
    } = input;
    let (group_dir, first_config, second_config) =
        config::prepare_gateway_configs(root, first_case, fixtures, &mock.base_url)?;
    let first_case_env = native_feature_eligibility::child_environment(root, first_case, true)?;
    let second_case_env =
        native_feature_eligibility::child_environment(root, first_case, options.self_check)?;
    let first_input = GatewayInput {
        options,
        group_dir: &group_dir,
        config_path: &first_config,
        case_env: &first_case_env,
    };
    let second_input = GatewayInput {
        options,
        group_dir: &group_dir,
        config_path: &second_config,
        case_env: &second_case_env,
    };
    let client = http::client()?;
    let captures = capture_gateways(&first_input, &second_input, cases, &client, mock)?;
    compare_group(root, cases, &captures)
}

fn capture_gateways(
    first_input: &GatewayInput<'_>,
    second_input: &GatewayInput<'_>,
    cases: &[Case],
    client: &Client,
    mock: MockProvider,
) -> Result<GroupCapture, OracleError> {
    let first = start_gateway(first_input, true)?;
    let first_log = first.log_path.clone();
    let first_results = capture_gateway(client, first, cases)?;
    let first_trace = mock.finish_run()?;
    let second = start_gateway(second_input, false)?;
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
    let artifact_directory = artifacts::write(artifacts::ArtifactInput {
        result: result.as_ref().err(),
        ..input
    })?;
    native_feature_eligibility::write_marker(
        &artifact_directory,
        input.case.native_feature_eligibility,
    )?;
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
