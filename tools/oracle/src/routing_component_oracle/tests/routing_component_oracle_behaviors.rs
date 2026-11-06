use rstest::rstest;

use crate::router_probes::ProbeOutput;
use crate::router_probes::single_candidate::{Deployment, Input, Output};
use crate::router_probes::{ProbeOperation, Reply, Request, Sample, execute};
use crate::routing_probe_ipc::{ProbeClock, ProbeFailure, parse_message};

fn request(deployments: Vec<Deployment>, samples: Vec<u64>) -> Request {
    Request {
        clock: ProbeClock {
            unix_millis: 1_700_000_000_123,
            monotonic_millis: 42_000,
        },
        samples: samples
            .into_iter()
            .map(|value| Sample::try_from(value).unwrap())
            .collect(),
        operation: ProbeOperation::SingleCandidate(Input { deployments }),
    }
}

fn deployment(id: &str, weight: u32, rpm: u32, tpm: u32) -> Deployment {
    Deployment {
        id: id.to_owned(),
        weight,
        rpm,
        tpm,
    }
}

fn selected(id: &str) -> Reply {
    Reply::Ok {
        output: ProbeOutput::SingleCandidate(Output {
            selected: id.to_owned(),
        }),
        warnings: Vec::new(),
    }
}

#[rstest]
#[case::weight(3, 0, 0)]
#[case::rpm(0, 3, 0)]
#[case::tpm(0, 0, 3)]
fn a_zero_deployment_never_wins_over_the_metric(
    #[case] weight: u32,
    #[case] rpm: u32,
    #[case] tpm: u32,
) {
    let request = request(
        vec![
            deployment("zero", 0, 0, 0),
            deployment("only", weight, rpm, tpm),
        ],
        vec![0],
    );
    assert_eq!(execute(&request), selected("only"));
}

#[test]
fn one_candidate_is_selected_through_the_uniform_fallback() {
    let request = request(vec![deployment("only", 0, 0, 0)], vec![0]);
    assert_eq!(execute(&request), selected("only"));
}

#[test]
fn the_reply_names_the_operation_and_the_selected_deployment() {
    let request = request(vec![deployment("only", 3, 0, 0)], vec![0]);
    let reply = execute(&request);
    assert_eq!(
        serde_json::to_string(&reply).unwrap(),
        r#"{"outcome":"ok","output":{"single_candidate":{"selected":"only"}},"warnings":[]}"#
    );
}

#[test]
fn a_request_round_trips_through_the_wire_format() {
    let request = request(vec![deployment("only", 3, 0, 0)], vec![0]);
    let bytes = serde_json::to_vec(&request).unwrap();
    let parsed: Request = parse_message(&bytes).unwrap();
    assert_eq!(parsed, request);
}

#[test]
fn the_sample_at_the_draw_limit_is_accepted() {
    let request = request(
        vec![deployment("only", 0, 0, 0)],
        vec![(1_u64 << 53_u32) - 1],
    );
    assert_eq!(execute(&request), selected("only"));
}

#[test]
fn a_heavy_weight_candidate_answers_with_the_deployment_for_a_large_sample() {
    let request = request(vec![deployment("only", 1000, 0, 0)], vec![1_u64 << 52_u32]);
    assert_eq!(execute(&request), selected("only"));
}

#[test]
fn a_weighted_candidate_without_samples_answers_samples_exhausted() {
    let request = request(vec![deployment("only", 3, 0, 0)], Vec::new());
    assert_eq!(
        execute(&request),
        Reply::Error {
            kind: ProbeFailure::SamplesExhausted,
            message: "the request has no sample left for this draw".to_owned(),
        }
    );
}

#[test]
fn an_empty_deployment_list_is_an_invalid_request() {
    let request = request(Vec::new(), vec![0]);
    assert_eq!(
        execute(&request),
        Reply::Error {
            kind: ProbeFailure::InvalidRequest,
            message: "there are no deployments to choose from".to_owned(),
        }
    );
}
