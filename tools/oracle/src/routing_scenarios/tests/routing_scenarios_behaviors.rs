use std::time::{Duration, Instant};

use rstest::rstest;

use super::super::compare::{compare_responses, compare_transcript};
use super::super::run::run_scenario;
use super::super::{ConcurrentStep, HttpStep, JoinPoint, Scenario, ScenarioStep, scripted_replies};
use super::{answer, provider_request, request, response};
use crate::OracleError;
use crate::case::Compare;
use crate::http;
use crate::mock::MockProvider;
use crate::routing_mock_scripts::{MockScript, Outcome, ScriptSpec, ScriptedReply, Wait};

fn provider(deadline_ms: u64, replies: Vec<ScriptedReply>) -> MockProvider {
    MockProvider::start(
        MockScript::try_from(ScriptSpec {
            deadline_ms,
            replies,
        })
        .unwrap(),
    )
    .unwrap()
}

fn http_step(path: &str) -> HttpStep {
    HttpStep {
        request: request(path),
        compare: Compare::default(),
    }
}

#[test]
fn http_steps_run_sequentially_and_capture_each_response_in_order() {
    let mock = provider(
        5000,
        vec![
            ScriptedReply {
                wait: Wait::None,
                then: Outcome::Answer(answer(200, "first")),
            },
            ScriptedReply {
                wait: Wait::None,
                then: Outcome::Answer(answer(200, "second")),
            },
        ],
    );
    let scenario = Scenario {
        steps: vec![
            ScenarioStep::Http(http_step("/a")),
            ScenarioStep::Http(http_step("/b")),
        ],
    };
    let client = http::client().unwrap();
    let responses = run_scenario(&client, &mock.base_url, &scenario).unwrap();
    let tags = responses
        .iter()
        .map(|captured| captured.body["tag"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        tags,
        [serde_json::json!("first"), serde_json::json!("second")]
    );
}

#[test]
fn a_concurrent_step_dispatches_every_request_before_its_barrier_can_release() {
    let mock = provider(
        1000,
        vec![
            ScriptedReply {
                wait: Wait::Barrier {
                    name: "pair".to_owned(),
                },
                then: Outcome::Answer(answer(200, "pair")),
            },
            ScriptedReply {
                wait: Wait::Barrier {
                    name: "pair".to_owned(),
                },
                then: Outcome::Answer(answer(200, "pair")),
            },
        ],
    );
    let scenario = Scenario {
        steps: vec![
            ScenarioStep::Concurrent(ConcurrentStep {
                requests: vec![http_step("/a"), http_step("/b")],
            }),
            ScenarioStep::Join(JoinPoint { deadline_ms: 1000 }),
        ],
    };
    let client = http::client().unwrap();
    let responses = run_scenario(&client, &mock.base_url, &scenario).unwrap();
    assert_eq!(responses.len(), 2);
    assert!(responses.iter().all(|captured| captured.status == 200));
}

#[rstest]
#[case::matching_responses(response(200, "second"), true)]
#[case::altered_second_response(response(200, "tampered"), false)]
fn compare_responses_detects_an_altered_second_response(
    #[case] second_reply: http::Response,
    #[case] matches: bool,
) {
    let steps = [http_step("/a"), http_step("/b")];
    let references = [&steps[0], &steps[1]];
    let first = [response(200, "first"), response(200, "second")];
    let second = [response(200, "first"), second_reply];
    assert_eq!(
        compare_responses(&references, &first, &second).is_ok(),
        matches
    );
}

#[rstest]
#[case::matching_trace("first", true)]
#[case::altered_provider_trace("tampered", false)]
fn compare_transcript_detects_an_altered_provider_trace(
    #[case] second_value: &str,
    #[case] matches: bool,
) {
    let mut step = http_step("/a");
    step.compare.upstream_headers = vec!["x-trace".to_owned()];
    let steps = [&step];
    let first_requests = [provider_request("x-trace", "first")];
    let second_requests = [provider_request("x-trace", second_value)];
    assert_eq!(
        compare_transcript(&steps, &first_requests, &second_requests).is_ok(),
        matches
    );
}

#[test]
fn a_concurrent_step_with_a_reply_still_pending_times_out_at_the_join_deadline() {
    let mock = provider(
        5000,
        vec![
            ScriptedReply {
                wait: Wait::None,
                then: Outcome::Answer(answer(200, "fast")),
            },
            ScriptedReply {
                wait: Wait::Delay { millis: 3000 },
                then: Outcome::Answer(answer(200, "slow")),
            },
        ],
    );
    let scenario = Scenario {
        steps: vec![
            ScenarioStep::Concurrent(ConcurrentStep {
                requests: vec![http_step("/a"), http_step("/b")],
            }),
            ScenarioStep::Join(JoinPoint { deadline_ms: 100 }),
        ],
    };
    let client = http::client().unwrap();
    let elapsed = Instant::now();
    let result = run_scenario(&client, &mock.base_url, &scenario);
    let Err(OracleError::HttpRequest(failure)) = result else {
        panic!("expected a timed-out HTTP request, got {result:?}")
    };
    assert!(failure.is_timeout());
    assert!(elapsed.elapsed() < Duration::from_secs(1));
}

#[test]
fn scripted_replies_keeps_every_upstream_answer_for_a_retry() {
    let scenario = Scenario {
        steps: vec![ScenarioStep::Http(http_step("/a"))],
    };
    let upstream = vec![answer(429, "first"), answer(200, "second")];
    let replies = scripted_replies("retry", &scenario, &upstream).unwrap();
    let statuses = replies
        .iter()
        .map(|reply| match &reply.then {
            Outcome::Answer(answer) => answer.status,
            Outcome::Close => 0,
        })
        .collect::<Vec<_>>();
    assert_eq!(statuses, [429, 200]);
    assert!(replies.iter().all(|reply| reply.wait == Wait::None));
}

#[test]
fn scripted_replies_rejects_fewer_upstream_answers_than_requests() {
    let scenario = Scenario {
        steps: vec![
            ScenarioStep::Http(http_step("/a")),
            ScenarioStep::Http(http_step("/b")),
        ],
    };
    let upstream = vec![answer(200, "only")];
    let error = scripted_replies("short", &scenario, &upstream).unwrap_err();
    assert!(matches!(
        error,
        OracleError::ScenarioUpstreamShort {
            requests: 2,
            upstream: 1
        }
    ));
}
