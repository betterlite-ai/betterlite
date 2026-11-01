#![cfg(test)]

use super::{Input, Op};
use crate::helpers::Helper as _;

#[test]
fn empty_groups_leave_the_starting_history_untouched() {
    let input: Input = serde_json::from_str(r#"{"groups":[]}"#).unwrap();
    let output = Op::run(&input).unwrap();
    assert_eq!(output["steps"], serde_json::json!([]));
}

#[test]
fn an_equivalent_complete_observation_takes_the_identical_baseline_path() {
    let input: Input = serde_json::from_str(
        r#"{"groups":[[{
            "request_id": "r1",
            "started_at": 0.0,
            "available_at": 0.0,
            "outcome": "complete",
            "baseline_equivalent": true,
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 2,
                "total_tokens": 12,
                "prompt_tokens_details": {
                    "text_tokens": 10,
                    "cached_tokens": 0,
                    "cache_creation_tokens": 0
                }
            }
        }]]}"#,
    )
    .unwrap();
    let output = Op::run(&input).unwrap();
    let estimate = &output["steps"][0]["estimates"][0];
    assert_eq!(estimate["reason"], "identical_baseline_path");
    assert_eq!(estimate["provenance"], "observed_identical");
    assert_eq!(estimate["usage"]["prompt_tokens"], 10_i64);
    assert_eq!(output["steps"][0]["history"]["last_at"], 0.0_f64);
    assert_eq!(output["steps"][0]["history"]["equivalent"], true);
}
