use betterlite_postgres_fixture::connection::{DatabaseNameGenerator, RandomDatabaseName};
use reqwest::blocking::Client;
use secrecy::SecretString;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

pub(crate) use self::compare_cases::compare_group;
use super::gateway::{GatewayInput, start as start_gateway};
use crate::OracleError;
use crate::case::{Case, LoadedCase};
use crate::cli::Options;
use crate::helpers;
use crate::http::browser_flow;
use crate::http::{self, Response};
use crate::mock::{MockProvider, ProviderRequest};
use crate::paths::artifact_root;
use crate::process::Gateway;
use crate::routing_mock_scripts::MockScript;
use crate::websocket_driver::{compare_transcript, exchange_gateway, start_provider};
use crate::websocket_oracle::{MAX_EXECUTION_SECONDS, WebSocketCase};

pub(crate) mod browser_flow_group;
pub(crate) mod compare_cases;
mod config;
mod fixtures;
pub(crate) mod native_feature_eligibility;
mod scenario;

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
    let mut scenarios = Vec::new();
    let mut websocket_cases = Vec::new();
    let mut helper_outcome = helpers::HelperRun::Match;
    for loaded_case in cases {
        match loaded_case {
            LoadedCase::Http(http_case) => {
                let group_key = format!(
                    "{}\n{:?}\n{}",
                    http_case.config, http_case.env, http_case.native_feature_eligibility
                );
                groups.entry(group_key).or_default().push(http_case);
            }
            LoadedCase::Scenario(scenario_case) => scenarios.push(scenario_case),
            LoadedCase::Helper(helper_case) => {
                let result = helpers::run_case(&helper_case, options.self_check)?;
                if matches!(result, helpers::HelperRun::Difference) {
                    helper_outcome = helpers::HelperRun::Difference;
                }
            }
            LoadedCase::WebSocket(websocket_case) => websocket_cases.push(websocket_case),
        }
    }
    let root = artifact_root()?;
    let mut outcomes = Vec::new();
    for (_, group) in groups {
        outcomes.extend(run_group(options, &root, &group)?);
    }
    // Each scenario is its own gateway pair, never batched with other scenarios or Http cases.
    for scenario_case in &scenarios {
        outcomes.push(scenario::run_scenario_case(options, &root, scenario_case)?);
    }
    for websocket_case in &websocket_cases {
        if let Some(service) = &websocket_case.service {
            let helper_case = crate::case::HelperCase {
                kind: crate::case::helper::HelperKind::Helper,
                name: websocket_case.name.clone(),
                call: helpers::HelperCall {
                    operation: "websocket_oracle".to_owned(),
                    input: serde_json::json!({"service": service, "input": websocket_case.input}),
                },
                env: websocket_case.env.clone(),
                compare: crate::case::helper::HelperCompare::default(),
            };
            let result = helpers::run_case(&helper_case, options.self_check)?;
            if matches!(result, helpers::HelperRun::Difference) {
                helper_outcome = helpers::HelperRun::Difference;
            }
        } else {
            outcomes.extend(run_websocket_gateway_case(options, &root, websocket_case)?);
        }
    }
    if matches!(helper_outcome, helpers::HelperRun::Difference) {
        return Ok(exit_status(true));
    }
    Ok(exit_status(outcomes.contains(&true)))
}

struct WebSocketGatewayAttempt<'input> {
    options: &'input Options,
    root: &'input Path,
    case: &'input WebSocketCase,
    runtime: &'input tokio::runtime::Runtime,
    first: bool,
}

fn run_websocket_gateway_case(
    options: &Options,
    root: &Path,
    case: &WebSocketCase,
) -> Result<Vec<bool>, OracleError> {
    let group_dir = root
        .join("websocket")
        .join(crate::paths::safe_name(&case.name));
    fs::create_dir_all(&group_dir).map_err(|source| OracleError::Filesystem {
        path: group_dir.clone(),
        source,
    })?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| OracleError::ScenarioInvalid {
            message: source.to_string(),
        })?;
    let first = run_websocket_gateway_attempt(WebSocketGatewayAttempt {
        options,
        root,
        case,
        runtime: &runtime,
        first: true,
    })?;
    let second = run_websocket_gateway_attempt(WebSocketGatewayAttempt {
        options,
        root,
        case,
        runtime: &runtime,
        first: false,
    })?;
    Ok(vec![first, second])
}

fn run_websocket_gateway_attempt(
    attempt: WebSocketGatewayAttempt<'_>,
) -> Result<bool, OracleError> {
    let WebSocketGatewayAttempt {
        options,
        root,
        case,
        runtime,
        first,
    } = attempt;
    let provider = runtime
        .block_on(start_provider(case.clone()))
        .map_err(|error| OracleError::ScenarioInvalid {
            message: error.to_string(),
        })?;
    let provider_url = format!("ws://{}", provider.address);
    let default_config = "model_list:\n  - model_name: realtime-a\n    litellm_params:\n      model: openai/realtime-a\n      api_base: __WEBSOCKET_PROVIDER__\n      api_key: sk-provider\n  - model_name: gpt-realtime-whisper\n    litellm_params:\n      model: openai/gpt-realtime-whisper\n      api_base: __WEBSOCKET_PROVIDER__\n      api_key: sk-provider\n";
    let template = if case.config.is_empty() {
        default_config
    } else {
        &case.config
    };
    let config = template.replace("__WEBSOCKET_PROVIDER__", &provider_url);
    let config_path = root.join("websocket").join(format!(
        "{}-{}.yaml",
        crate::paths::safe_name(&case.name),
        first
    ));
    if let Err(source) = fs::write(&config_path, config) {
        let _cancelled = runtime.block_on(provider.cancel());
        return Err(OracleError::Filesystem {
            path: config_path,
            source,
        });
    }
    let input = GatewayInput {
        options,
        group_dir: root,
        config_path: &config_path,
        case_env: &case.env,
    };
    let mut gateway = match start_gateway(&input, first) {
        Ok(gateway) => gateway,
        Err(error) => {
            let _cancelled = runtime.block_on(provider.cancel());
            return Err(error);
        }
    };
    let captured = runtime.block_on(exchange_gateway(&gateway.base_url, case));
    let stop_result = gateway.stop();
    let observed = match captured {
        Ok(observed) => observed,
        Err(error) => {
            let _cancelled = runtime.block_on(provider.cancel());
            stop_result?;
            return Err(OracleError::ScenarioInvalid {
                message: error.to_string(),
            });
        }
    };
    stop_result?;
    let provider_observation = if observed.handshake.status == 101 {
        Some(
            runtime
                .block_on(provider.finish(Duration::from_secs(MAX_EXECUTION_SECONDS)))
                .map_err(|error| OracleError::ScenarioInvalid {
                    message: error.to_string(),
                })?,
        )
    } else {
        runtime
            .block_on(provider.cancel())
            .map_err(|error| OracleError::ScenarioInvalid {
                message: error.to_string(),
            })?;
        None
    };
    let mut difference = compare_transcript(case, &observed).is_err();
    if let Some(provider_observation) = provider_observation {
        difference |= provider_observation.connection_count != 1;
        difference |= provider_observation
            .request_uri
            .split_once('?')
            .map(|(_, query)| query)
            .unwrap_or("")
            != case.expected_upstream_query.as_deref().unwrap_or("");
        difference |=
            crate::websocket_oracle::direction_matches(&case.frames, &provider_observation.frames)
                .is_err();
    }
    Ok(difference)
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
    run_group_with_url(
        options,
        root,
        cases,
        environment.postgres_test_url.as_ref(),
        &mut RandomDatabaseName,
    )
}

pub(crate) fn run_group_with_url<G>(
    options: &Options,
    root: &Path,
    cases: &[Case],
    configured_url: Option<&SecretString>,
    generator: &mut G,
) -> Result<Vec<bool>, OracleError>
where
    G: DatabaseNameGenerator,
{
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
    let fixtures = fixtures::provision_with_generator(cases, configured_url, generator)?;
    let operation_result = run_group_capture(GroupCaptureInput {
        options,
        root,
        cases,
        first_case,
        fixtures: &fixtures,
        mock,
    });
    with_fixture_cleanup(fixtures, operation_result)
}

// Shared by both pipelines: provision a fixture, run once, then close it regardless of the outcome.
fn with_fixture_cleanup<T>(
    fixtures: fixtures::FixtureGuard,
    operation_result: Result<T, OracleError>,
) -> Result<T, OracleError> {
    let cleanup_result = fixtures.close();
    match (operation_result, cleanup_result) {
        (Ok(value), Ok(())) => Ok(value),
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
    let captures = capture_gateways(
        &first_input,
        &second_input,
        &client,
        mock,
        |http_client, gateway| capture_gateway(http_client, gateway, cases),
    )?;
    compare_group(root, cases, &captures)
}

// Shared by the Http and Scenario pipelines: start a gateway, capture against it, then the next.
fn capture_gateways<F>(
    first_input: &GatewayInput<'_>,
    second_input: &GatewayInput<'_>,
    client: &Client,
    mock: MockProvider,
    mut capture: F,
) -> Result<GroupCapture, OracleError>
where
    F: FnMut(&Client, Gateway) -> Result<Vec<Response>, OracleError>,
{
    let first = start_gateway(first_input, true)?;
    let first_log = first.log_path.clone();
    let first_results = capture(client, first)?;
    let first_trace = mock.finish_run()?;
    let second = start_gateway(second_input, false)?;
    let second_log = second.log_path.clone();
    let second_results = capture(client, second)?;
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
