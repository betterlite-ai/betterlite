use std::collections::BTreeMap;
use std::path::Path;

use super::{HttpStep, Scenario, ScenarioStep};
use crate::OracleError;
use crate::artifacts;
use crate::case::{Compare, ScenarioCase};
use crate::compare::{self, Difference};
use crate::http::Response;
use crate::mock::ProviderRequest;
use crate::runner::native_feature_eligibility;

pub(crate) struct ScenarioCompareInput<'input> {
    pub(crate) root: &'input Path,
    pub(crate) case: &'input ScenarioCase,
    pub(crate) scenario: &'input Scenario,
    pub(crate) first: &'input [Response],
    pub(crate) second: &'input [Response],
    pub(crate) first_requests: &'input [ProviderRequest],
    pub(crate) second_requests: &'input [ProviderRequest],
    pub(crate) first_log: &'input Path,
    pub(crate) second_log: &'input Path,
}

pub(crate) fn flatten(scenario: &Scenario) -> Vec<&HttpStep> {
    scenario
        .steps
        .iter()
        .flat_map(|step| match step {
            ScenarioStep::Http(http_step) => vec![http_step],
            ScenarioStep::Concurrent(concurrent) => concurrent.requests.iter().collect(),
            ScenarioStep::Join(_join) => Vec::new(),
        })
        .collect()
}

pub(crate) fn compare_scenario_case(input: &ScenarioCompareInput<'_>) -> Result<bool, OracleError> {
    let steps = flatten(input.scenario);
    let result = compare_responses(&steps, input.first, input.second)
        .and_then(|()| compare_transcript(&steps, input.first_requests, input.second_requests));
    let last = steps.len().saturating_sub(1);
    let first_last = input
        .first
        .get(last)
        .ok_or(OracleError::MissingCapture(last))?;
    let second_last = input
        .second
        .get(last)
        .ok_or(OracleError::MissingCapture(last))?;
    let case = input.case.as_case();
    let artifact_directory = artifacts::write(artifacts::ArtifactInput {
        root: input.root,
        case: &case,
        first: first_last,
        second: second_last,
        first_requests: input.first_requests,
        second_requests: input.second_requests,
        first_log: input.first_log,
        second_log: input.second_log,
        result: result.as_ref().err(),
    })?;
    native_feature_eligibility::write_marker(
        &artifact_directory,
        input.case.native_feature_eligibility,
    )?;
    Ok(result.is_err())
}

pub(super) fn compare_responses(
    steps: &[&HttpStep],
    first: &[Response],
    second: &[Response],
) -> Result<(), Difference> {
    for (index, step) in steps.iter().enumerate() {
        let missing = || Difference {
            message: "a scenario response was not captured".to_owned(),
            pointer: format!("/steps/{index}"),
        };
        let left = first.get(index).ok_or_else(missing)?;
        let right = second.get(index).ok_or_else(missing)?;
        compare::compare(left, right, &[], &[], &step.compare).map_err(|difference| {
            Difference {
                message: difference.message,
                pointer: format!("/steps/{index}{}", difference.pointer),
            }
        })?;
    }
    Ok(())
}

pub(super) fn compare_transcript(
    steps: &[&HttpStep],
    first_requests: &[ProviderRequest],
    second_requests: &[ProviderRequest],
) -> Result<(), Difference> {
    let mut upstream_headers = steps
        .iter()
        .flat_map(|step| step.compare.upstream_headers.iter().cloned())
        .collect::<Vec<_>>();
    upstream_headers.sort();
    upstream_headers.dedup();
    let sentinel = Response {
        status: 0,
        headers: BTreeMap::new(),
        body: serde_json::Value::Null,
    };
    let transcript_compare = Compare {
        headers: Vec::new(),
        headers_present: Vec::new(),
        upstream_headers,
        ignore: Vec::new(),
    };
    compare::compare(
        &sentinel,
        &sentinel,
        first_requests,
        second_requests,
        &transcript_compare,
    )
}
