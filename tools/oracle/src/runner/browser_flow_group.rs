use std::collections::BTreeMap;
use std::path::Path;

use super::{GroupCapture, compare_case, native_feature_eligibility, request_slice};
use crate::OracleError;
use crate::artifacts;
use crate::case::Case;
use crate::case::Compare;
use crate::case::browser_flow::Step;
use crate::compare;
use crate::http::Response;
use crate::mock::ProviderRequest;

struct CaseSlices<'slices> {
    first_responses: &'slices [Response],
    second_responses: &'slices [Response],
    first_requests: &'slices [ProviderRequest],
    second_requests: &'slices [ProviderRequest],
}

fn case_slices<'slices>(
    case: &Case,
    captures: &'slices GroupCapture,
    response_offset: usize,
    request_offset: usize,
) -> CaseSlices<'slices> {
    let response_count = case.steps.as_ref().map_or(1, Vec::len);
    CaseSlices {
        first_responses: response_slice(&captures.first, response_offset, response_count),
        second_responses: response_slice(&captures.second, response_offset, response_count),
        first_requests: request_slice(
            &captures.first_requests,
            request_offset,
            case.upstream.len(),
        ),
        second_requests: request_slice(
            &captures.second_requests,
            request_offset,
            case.upstream.len(),
        ),
    }
}

pub(crate) fn compare_one_case(
    root: &Path,
    case: &Case,
    captures: &GroupCapture,
    response_offset: usize,
    request_offset: usize,
) -> Result<bool, OracleError> {
    let slices = case_slices(case, captures, response_offset, request_offset);
    if let Some(steps) = &case.steps {
        compare_case_steps(StepsCompareInput {
            root,
            case,
            steps,
            first: slices.first_responses,
            second: slices.second_responses,
            first_requests: slices.first_requests,
            second_requests: slices.second_requests,
            first_log: &captures.first_log,
            second_log: &captures.second_log,
        })
    } else {
        let first = slices
            .first_responses
            .first()
            .ok_or(OracleError::MissingCapture(response_offset))?;
        let second = slices
            .second_responses
            .first()
            .ok_or(OracleError::MissingCapture(response_offset))?;
        compare_case(artifacts::ArtifactInput {
            root,
            case,
            first,
            second,
            first_requests: slices.first_requests,
            second_requests: slices.second_requests,
            first_log: &captures.first_log,
            second_log: &captures.second_log,
            result: None,
        })
    }
}

fn response_slice(responses: &[Response], offset: usize, count: usize) -> &[Response] {
    let end = offset.saturating_add(count);
    responses.get(offset..end).unwrap_or_default()
}

#[derive(Clone, Copy)]
pub(crate) struct StepsCompareInput<'input> {
    pub(crate) root: &'input Path,
    pub(crate) case: &'input Case,
    pub(crate) steps: &'input [Step],
    pub(crate) first: &'input [Response],
    pub(crate) second: &'input [Response],
    pub(crate) first_requests: &'input [ProviderRequest],
    pub(crate) second_requests: &'input [ProviderRequest],
    pub(crate) first_log: &'input Path,
    pub(crate) second_log: &'input Path,
}

// The transcript compares once more through compare::compare with a sentinel response pair.
pub(crate) fn compare_case_steps(input: StepsCompareInput<'_>) -> Result<bool, OracleError> {
    let result = compare_steps(input.steps, input.first, input.second).and_then(|()| {
        compare_transcript(input.steps, input.first_requests, input.second_requests)
    });
    let last = input.steps.len().saturating_sub(1);
    let first_last = input
        .first
        .get(last)
        .ok_or(OracleError::MissingCapture(last))?;
    let second_last = input
        .second
        .get(last)
        .ok_or(OracleError::MissingCapture(last))?;
    let artifact_directory = artifacts::write(artifacts::ArtifactInput {
        root: input.root,
        case: input.case,
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

fn compare_steps(
    steps: &[Step],
    first: &[Response],
    second: &[Response],
) -> Result<(), compare::Difference> {
    for (index, step) in steps.iter().enumerate() {
        let missing = || compare::Difference {
            message: "a step's response was not captured".to_owned(),
            pointer: format!("/steps/{index}"),
        };
        let left = first.get(index).ok_or_else(missing)?;
        let right = second.get(index).ok_or_else(missing)?;
        compare::compare(left, right, &[], &[], &step.compare)?;
    }
    Ok(())
}

fn compare_transcript(
    steps: &[Step],
    first_requests: &[ProviderRequest],
    second_requests: &[ProviderRequest],
) -> Result<(), compare::Difference> {
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
