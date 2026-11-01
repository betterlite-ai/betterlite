use reqwest::blocking::Client;
use secrecy::SecretString;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

pub(crate) use self::compare_cases::{compare_group, compare_group_cases};
use super::gateway::{GatewayInput, start as start_gateway};
use crate::OracleError;
use crate::case::{Case, GroupCase, LoadedCase};
use crate::cli::Options;
use crate::helpers;
use crate::http::browser_flow;
use crate::http::{self, Response};
use crate::mock::{MockProvider, ProviderRequest};
use crate::paths::artifact_root;
use crate::process::Gateway;
use crate::routing_mock_scripts::{MockScript, Outcome, ScriptSpec, ScriptedReply, Wait};
use crate::routing_scenarios;

pub(crate) mod browser_flow_group;
pub(crate) mod compare_cases;
mod config;
mod fixtures;
pub(crate) mod native_feature_eligibility;

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
    let (groups, helper_outcome) = group_cases(options, cases)?;
    let root = artifact_root()?;
    let mut outcomes = Vec::new();
    for group in groups.values() {
        outcomes.extend(run_one_group(options, &root, group)?);
    }
    if matches!(helper_outcome, helpers::HelperRun::Difference) {
        return Ok(exit_status(true));
    }
    Ok(exit_status(outcomes.contains(&true)))
}

// Helper cases run and compare themselves inline, so only Http and Scenario cases need a group.
fn group_cases(
    options: &Options,
    cases: Vec<LoadedCase>,
) -> Result<(BTreeMap<String, Vec<GroupCase>>, helpers::HelperRun), OracleError> {
    let mut groups = BTreeMap::<String, Vec<GroupCase>>::new();
    let mut helper_outcome = helpers::HelperRun::Match;
    for loaded_case in cases {
        let group_case = match loaded_case {
            LoadedCase::Http(http_case) => GroupCase::Http(http_case),
            LoadedCase::Scenario(scenario_case) => GroupCase::Scenario(scenario_case),
            LoadedCase::Helper(helper_case) => {
                let result = helpers::run_case(&helper_case, options.self_check)?;
                if matches!(result, helpers::HelperRun::Difference) {
                    helper_outcome = helpers::HelperRun::Difference;
                }
                continue;
            }
        };
        let group_key = format!(
            "{}\n{:?}\n{}",
            group_case.config(),
            group_case.env(),
            group_case.native_feature_eligibility()
        );
        groups.entry(group_key).or_default().push(group_case);
    }
    Ok((groups, helper_outcome))
}

// A pure-Http group runs through the plain Case pipeline; a scenario group needs the mock script's per-step waits.
fn run_one_group(
    options: &Options,
    root: &Path,
    group: &[GroupCase],
) -> Result<Vec<bool>, OracleError> {
    if let Some(http_cases) = GroupCase::all_http(group) {
        return run_group(options, root, &http_cases);
    }
    let environment =
        <DatabaseEnvironment as clap::Parser>::try_parse_from([env!("CARGO_PKG_NAME")])?;
    run_group_cases(options, root, group, environment.postgres_test_url.as_ref())
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
    let group_cases: Vec<GroupCase> = cases.iter().cloned().map(GroupCase::Http).collect();
    run_group_cases(options, root, &group_cases, configured_url)
}

fn run_group_cases(
    options: &Options,
    root: &Path,
    cases: &[GroupCase],
    configured_url: Option<&SecretString>,
) -> Result<Vec<bool>, OracleError> {
    if cases.is_empty() {
        return Err(OracleError::MissingCapture(0));
    }
    let mock = MockProvider::start(MockScript::try_from(ScriptSpec {
        deadline_ms: routing_scenarios::MAX_DEADLINE_MS,
        replies: scripted_replies(cases),
    })?)?;
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

fn scripted_replies(cases: &[GroupCase]) -> Vec<ScriptedReply> {
    cases
        .iter()
        .flat_map(|case| {
            let waits = match case {
                GroupCase::Http(http_case) => vec![Wait::None; http_case.upstream.len()],
                GroupCase::Scenario(scenario_case) => {
                    routing_scenarios::upstream_waits(&scenario_case.name, &scenario_case.scenario)
                }
            };
            waits.into_iter().zip(case.upstream().iter().cloned())
        })
        .map(|(wait, answer)| ScriptedReply {
            wait,
            then: Outcome::Answer(answer),
        })
        .collect()
}

struct GroupCaptureInput<'input> {
    options: &'input Options,
    root: &'input Path,
    cases: &'input [GroupCase],
    first_case: &'input GroupCase,
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
    let first_case_env =
        native_feature_eligibility::child_environment(first_case, true, &mock.base_url);
    let second_case_env = native_feature_eligibility::child_environment(
        first_case,
        options.self_check,
        &mock.base_url,
    );
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
    if let Some(http_cases) = GroupCase::all_http(cases) {
        return compare_group(root, &http_cases, &captures);
    }
    compare_group_cases(root, cases, &captures)
}

fn capture_gateways(
    first_input: &GatewayInput<'_>,
    second_input: &GatewayInput<'_>,
    cases: &[GroupCase],
    client: &Client,
    mock: MockProvider,
) -> Result<GroupCapture, OracleError> {
    let first = start_gateway(first_input, true)?;
    let first_log = first.log_path.clone();
    let first_results = capture_gateway_cases(client, first, cases)?;
    let first_trace = mock.finish_run()?;
    let second = start_gateway(second_input, false)?;
    let second_log = second.log_path.clone();
    let second_results = capture_gateway_cases(client, second, cases)?;
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

pub(crate) fn capture_gateway(
    client: &Client,
    mut gateway: Gateway,
    cases: &[Case],
) -> Result<Vec<Response>, OracleError> {
    let mut captures = Vec::new();
    for case in cases {
        if let Some(steps) = &case.steps {
            captures.extend(browser_flow::run_case_steps(
                client,
                &gateway.base_url,
                steps,
            )?);
        } else {
            captures.push(http::send(client, &gateway.base_url, &case.request)?);
        }
    }
    gateway.stop()?;
    Ok(captures)
}

fn capture_gateway_cases(
    client: &Client,
    mut gateway: Gateway,
    cases: &[GroupCase],
) -> Result<Vec<Response>, OracleError> {
    if let Some(http_cases) = GroupCase::all_http(cases) {
        return capture_gateway(client, gateway, &http_cases);
    }
    let mut captures = Vec::new();
    for case in cases {
        match case {
            GroupCase::Http(http_case) => {
                if let Some(steps) = &http_case.steps {
                    captures.extend(browser_flow::run_case_steps(
                        client,
                        &gateway.base_url,
                        steps,
                    )?);
                } else {
                    captures.push(http::send(client, &gateway.base_url, &http_case.request)?);
                }
            }
            GroupCase::Scenario(scenario_case) => {
                captures.extend(routing_scenarios::run::run_scenario(
                    client,
                    &gateway.base_url,
                    &scenario_case.scenario,
                )?);
            }
        }
    }
    gateway.stop()?;
    Ok(captures)
}

pub(crate) fn request_slice(
    requests: &[ProviderRequest],
    offset: usize,
    count: usize,
) -> &[ProviderRequest] {
    let end = offset.saturating_add(count);
    requests.get(offset..end).unwrap_or_default()
}
