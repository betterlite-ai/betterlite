use rstest::rstest;

use super::support::{Case, LimitValue, Outcome, Source, Spec, check};
use crate::latency_rank_math::valid_count;

#[rstest]
// the token estimate is added first, and a sum exactly on the limit still fits
#[case::tokens_on_the_limit_fit(
    Case {
        specs: &[Spec {
            limits: &[("tpm", Source::Top, LimitValue::Value(10))],
            ..Spec::stored("a", &[1.0_f64], &[], (6, 0))
        }],
        input_tokens: 4,
        outcome: Outcome::Ranked {
            scores: &[1.0_f64],
            candidates: &["a"],
            valid: &["a"],
            selected: Some("a"),
        },
        ..Case::DEFAULT
    }
)]
#[case::one_token_over_the_limit_is_skipped(
    Case {
        specs: &[Spec {
            limits: &[("tpm", Source::Top, LimitValue::Value(10))],
            ..Spec::stored("a", &[1.0_f64], &[], (6, 0))
        }],
        input_tokens: 5,
        outcome: Outcome::Ranked {
            scores: &[1.0_f64],
            candidates: &[],
            valid: &[],
            selected: None,
        },
        ..Case::DEFAULT
    }
)]
// the request's own count is one more than the requests already in the minute
#[case::requests_on_the_limit_fit(
    Case {
        specs: &[Spec {
            limits: &[("rpm", Source::Top, LimitValue::Value(3))],
            ..Spec::stored("a", &[1.0_f64], &[], (0, 2))
        }],
        outcome: Outcome::Ranked {
            scores: &[1.0_f64],
            candidates: &["a"],
            valid: &["a"],
            selected: Some("a"),
        },
        ..Case::DEFAULT
    }
)]
#[case::next_request_over_the_limit_is_skipped(
    Case {
        specs: &[Spec {
            limits: &[("rpm", Source::Top, LimitValue::Value(3))],
            ..Spec::stored("a", &[1.0_f64], &[], (0, 3))
        }],
        outcome: Outcome::Ranked {
            scores: &[1.0_f64],
            candidates: &[],
            valid: &[],
            selected: None,
        },
        ..Case::DEFAULT
    }
)]
// a score exactly on lowest + lowest * buffer is still buffered
#[case::buffer_edge_is_inclusive(
    Case {
        specs: &[
            Spec::stored("a", &[10.0_f64], &[], (0, 0)),
            Spec::stored("b", &[15.0_f64], &[], (0, 0)),
        ],
        buffer: 0.5_f64,
        outcome: Outcome::Ranked {
            scores: &[10.0_f64, 15.0_f64],
            candidates: &["a", "b"],
            valid: &["a", "b"],
            selected: Some("a"),
        },
        ..Case::DEFAULT
    }
)]
#[case::buffer_edge_just_outside(
    Case {
        specs: &[
            Spec::stored("a", &[10.0_f64], &[], (0, 0)),
            Spec::stored("b", &[15.5_f64], &[], (0, 0)),
        ],
        buffer: 0.5_f64,
        outcome: Outcome::Ranked {
            scores: &[10.0_f64, 15.5_f64],
            candidates: &["a", "b"],
            valid: &["a"],
            selected: Some("a"),
        },
        ..Case::DEFAULT
    }
)]
#[case::equal_scores_stay_in_pool_order(
    Case {
        specs: &[
            Spec::stored("a", &[10.0_f64], &[], (0, 0)),
            Spec::stored("b", &[10.0_f64], &[], (0, 0)),
        ],
        draw: u64::MAX,
        outcome: Outcome::Ranked {
            scores: &[10.0_f64, 10.0_f64],
            candidates: &["a", "b"],
            valid: &["a", "b"],
            selected: Some("b"),
        },
        ..Case::DEFAULT
    }
)]
// a negative buffer lowers the bar below the lowest score, so LiteLLM's random.choice has no sequence
#[case::negative_buffer_keeps_nothing(
    Case {
        specs: &[Spec::stored("a", &[10.0_f64], &[], (0, 0))],
        buffer: -0.5_f64,
        outcome: Outcome::Ranked {
            scores: &[10.0_f64],
            candidates: &["a"],
            valid: &[],
            selected: None,
        },
        ..Case::DEFAULT
    }
)]
// an explicit zero is the limit, not a missing one: a request with no tokens still fits, any other does not
#[case::zero_tpm_fits_a_zero_token_request(
    Case {
        specs: &[Spec {
            limits: &[("tpm", Source::Top, LimitValue::Value(0))],
            ..Spec::stored("a", &[1.0_f64], &[], (0, 0))
        }],
        outcome: Outcome::Ranked {
            scores: &[1.0_f64],
            candidates: &["a"],
            valid: &["a"],
            selected: Some("a"),
        },
        ..Case::DEFAULT
    }
)]
#[case::zero_tpm_skips_any_token(
    Case {
        specs: &[Spec {
            limits: &[("tpm", Source::Top, LimitValue::Value(0))],
            ..Spec::stored("a", &[1.0_f64], &[], (0, 0))
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
// a null limit is no limit: the usage already recorded in the minute still fits
#[case::null_tpm_is_unbounded(
    Case {
        specs: &[Spec {
            limits: &[("tpm", Source::Top, LimitValue::Null)],
            ..Spec::stored("a", &[1.0_f64], &[], (5, 0))
        }],
        input_tokens: 5,
        outcome: Outcome::Ranked {
            scores: &[1.0_f64],
            candidates: &["a"],
            valid: &["a"],
            selected: Some("a"),
        },
        ..Case::DEFAULT
    }
)]
// the top-level tpm wins over litellm_params, which is the limit the params source alone would refuse
#[case::top_level_limit_wins(
    Case {
        specs: &[Spec {
            limits: &[
                ("tpm", Source::Top, LimitValue::Value(100)),
                ("tpm", Source::Params, LimitValue::Value(1)),
            ],
            ..Spec::stored("a", &[1.0_f64], &[], (0, 0))
        }],
        input_tokens: 50,
        outcome: Outcome::Ranked {
            scores: &[1.0_f64],
            candidates: &["a"],
            valid: &["a"],
            selected: Some("a"),
        },
        ..Case::DEFAULT
    }
)]
#[case::litellm_params_limit_wins_over_model_info(
    Case {
        specs: &[Spec {
            limits: &[
                ("tpm", Source::Params, LimitValue::Value(100)),
                ("tpm", Source::Info, LimitValue::Value(1)),
            ],
            ..Spec::stored("a", &[1.0_f64], &[], (0, 0))
        }],
        input_tokens: 50,
        outcome: Outcome::Ranked {
            scores: &[1.0_f64],
            candidates: &["a"],
            valid: &["a"],
            selected: Some("a"),
        },
        ..Case::DEFAULT
    }
)]
#[case::model_info_limit_is_read_last(
    Case {
        specs: &[Spec {
            limits: &[("tpm", Source::Info, LimitValue::Value(1))],
            ..Spec::stored("a", &[1.0_f64], &[], (0, 0))
        }],
        input_tokens: 50,
        outcome: Outcome::Ranked {
            scores: &[1.0_f64],
            candidates: &[],
            valid: &[],
            selected: None,
        },
        ..Case::DEFAULT
    }
)]
#[case::a_null_top_level_limit_falls_through(
    Case {
        specs: &[Spec {
            limits: &[
                ("tpm", Source::Top, LimitValue::Null),
                ("tpm", Source::Params, LimitValue::Value(100)),
            ],
            ..Spec::stored("a", &[1.0_f64], &[], (0, 0))
        }],
        input_tokens: 50,
        outcome: Outcome::Ranked {
            scores: &[1.0_f64],
            candidates: &["a"],
            valid: &["a"],
            selected: Some("a"),
        },
        ..Case::DEFAULT
    }
)]
#[case::a_limit_that_is_no_number_names_its_field(
    Case {
        specs: &[Spec {
            limits: &[("tpm", Source::Top, LimitValue::Text("5"))],
            ..Spec::stored("a", &[1.0_f64], &[], (0, 0))
        }],
        outcome: Outcome::Rejected { field: "tpm" },
        ..Case::DEFAULT
    }
)]
// only the bucket of the minute the selection reads counts
#[case::another_minute_is_not_read(
    Case {
        specs: &[Spec {
            limits: &[("tpm", Source::Top, LimitValue::Value(1))],
            ..Spec::stored("a", &[1.0_f64], &[], (100, 100))
        }],
        stored_minute: "2025-12-31-23-59",
        outcome: Outcome::Ranked {
            scores: &[1.0_f64],
            candidates: &["a"],
            valid: &["a"],
            selected: Some("a"),
        },
        ..Case::DEFAULT
    }
)]
// a percentile of one is the window's largest value
#[case::the_highest_percentile_reads_the_maximum(
    Case {
        specs: &[
            Spec::stored("a", &[], &[1.0_f64, 2.0_f64, 3.0_f64, 4.0_f64], (0, 0)),
            Spec::stored("b", &[], &[3.0_f64], (0, 0)),
        ],
        stream: true,
        percentile: Some(1.0_f64),
        outcome: Outcome::Ranked {
            scores: &[4.0_f64, 3.0_f64],
            candidates: &["b", "a"],
            valid: &["b"],
            selected: Some("b"),
        },
        ..Case::DEFAULT
    }
)]
#[case::an_empty_pool_has_no_deployment(
    Case {
        outcome: Outcome::Ranked {
            scores: &[],
            candidates: &[],
            valid: &[],
            selected: None,
        },
        ..Case::DEFAULT
    }
)]
fn latency_routing_boundaries(#[case] case: Case) {
    check(&case);
}

// the rank math directly: an empty ranking and a negative buffer keep nothing, and the edge is inclusive
#[test]
fn latency_routing_rank_math_boundaries() {
    assert_eq!(valid_count(&[], 0.5_f64), 0);
    assert_eq!(valid_count(&[10.0_f64], -0.5_f64), 0);
    assert_eq!(valid_count(&[10.0_f64], 0.0_f64), 1);
    assert_eq!(valid_count(&[10.0_f64, 10.0_f64], 0.0_f64), 2);
    assert_eq!(valid_count(&[10.0_f64, 15.0_f64], 0.5_f64), 2);
    assert_eq!(valid_count(&[10.0_f64, 15.1_f64], 0.5_f64), 1);
}
