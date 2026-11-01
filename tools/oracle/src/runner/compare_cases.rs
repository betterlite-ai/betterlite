use std::io::{self, Write as _};
use std::path::Path;

use super::{GroupCapture, browser_flow_group, native_feature_eligibility};
use crate::OracleError;
use crate::artifacts;
use crate::case::{Case, GroupCase, ScenarioCase};
use crate::compare;
use crate::http::Response;
use crate::mock::ProviderRequest;
use crate::routing_scenarios;

pub(crate) fn compare_group(
    root: &Path,
    cases: &[Case],
    captures: &GroupCapture,
) -> Result<Vec<bool>, OracleError> {
    let mut outcomes = Vec::with_capacity(cases.len());
    let mut request_offset = 0;
    let mut response_offset = 0;
    for case in cases {
        outcomes.push(compare_one_case(
            root,
            case,
            captures,
            response_offset,
            request_offset,
        )?);
        let response_count = case.steps.as_ref().map_or(1, Vec::len);
        response_offset = response_offset.saturating_add(response_count);
        request_offset = request_offset.saturating_add(case.upstream.len());
    }
    Ok(outcomes)
}

type CaseSlices<'slices> = (
    &'slices [Response],
    &'slices [Response],
    &'slices [ProviderRequest],
    &'slices [ProviderRequest],
);

// Both proxies answer the same case, so their slices share one offset pair.
fn case_slices<'slices>(
    case: &Case,
    captures: &'slices GroupCapture,
    response_offset: usize,
    request_offset: usize,
) -> CaseSlices<'slices> {
    let response_count = case.steps.as_ref().map_or(1, Vec::len);
    let response_end = response_offset.saturating_add(response_count);
    (
        captures
            .first
            .get(response_offset..response_end)
            .unwrap_or_default(),
        captures
            .second
            .get(response_offset..response_end)
            .unwrap_or_default(),
        super::request_slice(
            &captures.first_requests,
            request_offset,
            case.upstream.len(),
        ),
        super::request_slice(
            &captures.second_requests,
            request_offset,
            case.upstream.len(),
        ),
    )
}

// A steps-based case delegates to browser_flow_group; a single-request case compares directly.
fn compare_one_case(
    root: &Path,
    case: &Case,
    captures: &GroupCapture,
    response_offset: usize,
    request_offset: usize,
) -> Result<bool, OracleError> {
    let slices = case_slices(case, captures, response_offset, request_offset);
    if let Some(steps) = &case.steps {
        return browser_flow_group::compare_case_steps(browser_flow_group::StepsCompareInput {
            root,
            case,
            steps,
            first: slices.0,
            second: slices.1,
            first_requests: slices.2,
            second_requests: slices.3,
            first_log: &captures.first_log,
            second_log: &captures.second_log,
        });
    }
    compare_single_branch(root, case, captures, slices, response_offset)
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

fn compare_single_branch(
    root: &Path,
    case: &Case,
    captures: &GroupCapture,
    slices: CaseSlices<'_>,
    response_offset: usize,
) -> Result<bool, OracleError> {
    let (first_responses, second_responses, first_requests, second_requests) = slices;
    let first = first_responses
        .first()
        .ok_or(OracleError::MissingCapture(response_offset))?;
    let second = second_responses
        .first()
        .ok_or(OracleError::MissingCapture(response_offset))?;
    compare_case(artifacts::ArtifactInput {
        root,
        case,
        first,
        second,
        first_requests,
        second_requests,
        first_log: &captures.first_log,
        second_log: &captures.second_log,
        result: None,
    })
}

// A steps-based case contributes one response per step; a scenario contributes one per request it sends.
fn response_count(case: &GroupCase) -> usize {
    match case {
        GroupCase::Http(http_case) => http_case.steps.as_ref().map_or(1, Vec::len),
        GroupCase::Scenario(scenario_case) => {
            routing_scenarios::compare::flatten(&scenario_case.scenario).len()
        }
    }
}

// The caller already routes a pure-Http group through compare_group, so a group reaching here always carries a scenario case.
pub(crate) fn compare_group_cases(
    root: &Path,
    cases: &[GroupCase],
    captures: &GroupCapture,
) -> Result<Vec<bool>, OracleError> {
    let mut outcomes = Vec::with_capacity(cases.len());
    let mut request_offset = 0;
    let mut response_offset = 0;
    for case in cases {
        outcomes.push(compare_one_mixed_case(
            root,
            case,
            captures,
            response_offset,
            request_offset,
        )?);
        response_offset = response_offset.saturating_add(response_count(case));
        request_offset = request_offset.saturating_add(case.upstream().len());
    }
    Ok(outcomes)
}

fn compare_one_mixed_case(
    root: &Path,
    case: &GroupCase,
    captures: &GroupCapture,
    response_offset: usize,
    request_offset: usize,
) -> Result<bool, OracleError> {
    match case {
        GroupCase::Http(http_case) => {
            compare_one_case(root, http_case, captures, response_offset, request_offset)
        }
        GroupCase::Scenario(scenario_case) => compare_scenario_branch(
            root,
            scenario_case,
            captures,
            response_offset,
            request_offset,
        ),
    }
}

fn compare_scenario_branch(
    root: &Path,
    case: &ScenarioCase,
    captures: &GroupCapture,
    response_offset: usize,
    request_offset: usize,
) -> Result<bool, OracleError> {
    let response_end =
        response_offset.saturating_add(routing_scenarios::compare::flatten(&case.scenario).len());
    let first = captures
        .first
        .get(response_offset..response_end)
        .unwrap_or_default();
    let second = captures
        .second
        .get(response_offset..response_end)
        .unwrap_or_default();
    let first_requests = super::request_slice(
        &captures.first_requests,
        request_offset,
        case.upstream.len(),
    );
    let second_requests = super::request_slice(
        &captures.second_requests,
        request_offset,
        case.upstream.len(),
    );
    routing_scenarios::compare::compare_scenario_case(
        &routing_scenarios::compare::ScenarioCompareInput {
            root,
            case,
            scenario: &case.scenario,
            first,
            second,
            first_requests,
            second_requests,
            first_log: &captures.first_log,
            second_log: &captures.second_log,
        },
    )
}
