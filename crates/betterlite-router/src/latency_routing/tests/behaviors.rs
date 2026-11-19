use rstest::rstest;

use super::support::{Case, LimitValue, Outcome, Source, Spec, check};

#[rstest]
// an unused deployment enters the map with a one-sample zero window, so it outscores a recorded one
#[case::unseen_is_fastest(
    Case {
        specs: &[Spec::stored("seen", &[4.0_f64], &[], (0, 0)), Spec::unseen("fresh")],
        outcome: Outcome::Ranked {
            scores: &[4.0_f64, 0.0_f64],
            candidates: &["fresh", "seen"],
            valid: &["fresh"],
            selected: Some("fresh"),
        },
        ..Case::DEFAULT
    }
)]
// a non-streaming request scores the mean of the latency window
#[case::non_stream_mean(
    Case {
        specs: &[
            Spec::stored("a", &[1.0_f64, 3.0_f64], &[], (0, 0)),
            Spec::stored("b", &[4.0_f64, 8.0_f64], &[], (0, 0)),
        ],
        outcome: Outcome::Ranked {
            scores: &[2.0_f64, 6.0_f64],
            candidates: &["a", "b"],
            valid: &["a"],
            selected: Some("a"),
        },
        ..Case::DEFAULT
    }
)]
// a streaming request reads the first-token window, so the slower total latency does not matter
#[case::stream_first_token_mean(
    Case {
        specs: &[
            Spec::stored("a", &[10.0_f64], &[1.0_f64], (0, 0)),
            Spec::stored("b", &[2.0_f64], &[3.0_f64], (0, 0)),
        ],
        stream: true,
        outcome: Outcome::Ranked {
            scores: &[1.0_f64, 3.0_f64],
            candidates: &["a", "b"],
            valid: &["a"],
            selected: Some("a"),
        },
        ..Case::DEFAULT
    }
)]
// ttft_percentile reads the one-based ceil(len * percentile) rank of the first-token window
#[case::stream_first_token_percentile(
    Case {
        specs: &[
            Spec::stored("a", &[], &[1.0_f64, 2.0_f64, 3.0_f64, 4.0_f64], (0, 0)),
            Spec::stored("b", &[], &[2.5_f64], (0, 0)),
        ],
        stream: true,
        percentile: Some(0.5_f64),
        outcome: Outcome::Ranked {
            scores: &[2.0_f64, 2.5_f64],
            candidates: &["a", "b"],
            valid: &["a"],
            selected: Some("a"),
        },
        ..Case::DEFAULT
    }
)]
// a streaming request with no first-token sample falls back to the latency window
#[case::stream_without_first_token_samples(
    Case {
        specs: &[
            Spec::stored("a", &[5.0_f64], &[], (0, 0)),
            Spec::stored("b", &[1.0_f64], &[], (0, 0)),
        ],
        stream: true,
        outcome: Outcome::Ranked {
            scores: &[5.0_f64, 1.0_f64],
            candidates: &["b", "a"],
            valid: &["b"],
            selected: Some("b"),
        },
        ..Case::DEFAULT
    }
)]
// lowest_latency_buffer widens the valid set to lowest + lowest * buffer
#[case::buffer_keeps_a_near_deployment(
    Case {
        specs: &[
            Spec::stored("a", &[10.0_f64], &[], (0, 0)),
            Spec::stored("b", &[12.0_f64], &[], (0, 0)),
            Spec::stored("c", &[16.0_f64], &[], (0, 0)),
        ],
        buffer: 0.5_f64,
        outcome: Outcome::Ranked {
            scores: &[10.0_f64, 12.0_f64, 16.0_f64],
            candidates: &["a", "b", "c"],
            valid: &["a", "b"],
            selected: Some("a"),
        },
        ..Case::DEFAULT
    }
)]
// the draw reaches every buffered deployment, not only the first
#[case::buffer_draw_reaches_the_last_valid(
    Case {
        specs: &[
            Spec::stored("a", &[10.0_f64], &[], (0, 0)),
            Spec::stored("b", &[12.0_f64], &[], (0, 0)),
            Spec::stored("c", &[16.0_f64], &[], (0, 0)),
        ],
        buffer: 0.5_f64,
        draw: u64::MAX,
        outcome: Outcome::Ranked {
            scores: &[10.0_f64, 12.0_f64, 16.0_f64],
            candidates: &["a", "b", "c"],
            valid: &["a", "b"],
            selected: Some("b"),
        },
        ..Case::DEFAULT
    }
)]
// the fastest deployment has no room for this request's tokens, so the next lowest serves it
#[case::rate_limited_fastest_is_skipped(
    Case {
        specs: &[
            Spec {
                limits: &[("tpm", Source::Top, LimitValue::Value(10))],
                ..Spec::stored("fast", &[1.0_f64], &[], (10, 0))
            },
            Spec::stored("slow", &[5.0_f64], &[], (0, 0)),
        ],
        input_tokens: 5,
        outcome: Outcome::Ranked {
            scores: &[1.0_f64, 5.0_f64],
            candidates: &["slow"],
            valid: &["slow"],
            selected: Some("slow"),
        },
        ..Case::DEFAULT
    }
)]
// no deployment with room answers, and the pipeline turns the missing pick into no deployment
#[case::no_deployment_has_room(
    Case {
        specs: &[Spec {
            limits: &[("tpm", Source::Top, LimitValue::Value(0))],
            ..Spec::stored("full", &[1.0_f64], &[], (0, 0))
        }],
        input_tokens: 1,
        outcome: Outcome::Ranked {
            scores: &[1.0_f64],
            candidates: &[],
            valid: &[],
            selected: None,
        },
        ..Case::DEFAULT
    }
)]
fn latency_routing_behaviors(#[case] case: Case) {
    check(&case);
}
