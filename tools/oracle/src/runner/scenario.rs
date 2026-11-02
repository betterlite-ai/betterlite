use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::slice;

use super::{capture_gateways, config, fixtures, native_feature_eligibility, with_fixture_cleanup};
use crate::OracleError;
use crate::case::{Case, ScenarioCase};
use crate::cli::Options;
use crate::gateway::GatewayInput;
use crate::http;
use crate::mock::MockProvider;
use crate::routing_mock_scripts::{MockScript, ScriptSpec};
use crate::routing_scenarios;

// A scenario is always its own gateway pair, so there is no group key or multi-case bookkeeping.
pub(super) fn run_scenario_case(
    options: &Options,
    root: &Path,
    case: &ScenarioCase,
) -> Result<bool, OracleError> {
    let environment =
        <super::DatabaseEnvironment as clap::Parser>::try_parse_from([env!("CARGO_PKG_NAME")])?;
    let replies = routing_scenarios::scripted_replies(&case.name, &case.scenario, &case.upstream)?;
    let mock = MockProvider::start(MockScript::try_from(ScriptSpec {
        deadline_ms: routing_scenarios::MAX_DEADLINE_MS,
        replies,
    })?)?;
    let http_case = case.as_case();
    let fixtures = fixtures::provision(
        slice::from_ref(&http_case),
        environment.postgres_test_url.as_ref(),
    )?;
    let operation_result = capture_and_compare(ScenarioCaptureInput {
        options,
        root,
        case,
        fixtures: &fixtures,
        mock,
    });
    with_fixture_cleanup(fixtures, operation_result)
}

struct ScenarioCaptureInput<'input> {
    options: &'input Options,
    root: &'input Path,
    case: &'input ScenarioCase,
    fixtures: &'input fixtures::FixtureGuard,
    mock: MockProvider,
}

fn capture_and_compare(input: ScenarioCaptureInput<'_>) -> Result<bool, OracleError> {
    let ScenarioCaptureInput {
        options,
        root,
        case,
        fixtures,
        mock,
    } = input;
    let http_case = case.as_case();
    let scenario_config = prepare_scenario_config(
        root,
        &http_case,
        fixtures,
        &mock.base_url,
        options.self_check,
    )?;
    let (first_input, second_input) = gateway_input_pair(options, &scenario_config);
    let client = http::client()?;
    let captures = capture_gateways(
        &first_input,
        &second_input,
        &client,
        mock,
        |http_client, mut gateway| {
            let responses = routing_scenarios::run::run_scenario(
                http_client,
                &gateway.base_url,
                &case.scenario,
            )?;
            gateway.stop()?;
            Ok(responses)
        },
    )?;
    compare_scenario(root, case, &captures)
}

struct ScenarioConfig {
    group_dir: PathBuf,
    first_config: PathBuf,
    second_config: PathBuf,
    first_case_env: BTreeMap<String, String>,
    second_case_env: BTreeMap<String, String>,
}

fn prepare_scenario_config(
    root: &Path,
    http_case: &Case,
    fixtures: &fixtures::FixtureGuard,
    mock_base_url: &str,
    self_check: bool,
) -> Result<ScenarioConfig, OracleError> {
    let (group_dir, first_config, second_config) =
        config::prepare_gateway_configs(root, http_case, fixtures, mock_base_url)?;
    let first_case_env =
        native_feature_eligibility::child_environment(http_case, true, mock_base_url);
    let second_case_env =
        native_feature_eligibility::child_environment(http_case, self_check, mock_base_url);
    Ok(ScenarioConfig {
        group_dir,
        first_config,
        second_config,
        first_case_env,
        second_case_env,
    })
}

fn gateway_input_pair<'input>(
    options: &'input Options,
    scenario_config: &'input ScenarioConfig,
) -> (GatewayInput<'input>, GatewayInput<'input>) {
    let first_input = GatewayInput {
        options,
        group_dir: &scenario_config.group_dir,
        config_path: &scenario_config.first_config,
        case_env: &scenario_config.first_case_env,
    };
    let second_input = GatewayInput {
        options,
        group_dir: &scenario_config.group_dir,
        config_path: &scenario_config.second_config,
        case_env: &scenario_config.second_case_env,
    };
    (first_input, second_input)
}

fn compare_scenario(
    root: &Path,
    case: &ScenarioCase,
    captures: &super::GroupCapture,
) -> Result<bool, OracleError> {
    routing_scenarios::compare::compare_scenario_case(
        &routing_scenarios::compare::ScenarioCompareInput {
            root,
            case,
            scenario: &case.scenario,
            first: &captures.first,
            second: &captures.second,
            first_requests: &captures.first_requests,
            second_requests: &captures.second_requests,
            first_log: &captures.first_log,
            second_log: &captures.second_log,
        },
    )
}
