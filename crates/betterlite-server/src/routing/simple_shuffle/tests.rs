#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::iter;
use std::ops::RangeInclusive;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_openai::chat::execution::{ChatError, ChatExecutor};
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;

use super::pick_deployment;
use crate::server::chat_setup::{ChatSetupError, chat_executor};

fn config(router_settings: &str, params: &[&str]) -> Config {
    let entries: String = params
        .iter()
        .map(|extra| {
            [
                "  - model_name: g\n    litellm_params:\n      model: openai/",
                extra,
            ]
            .concat()
        })
        .collect();
    ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&format!("{router_settings}model_list:\n{entries}"))
    .unwrap()
}

fn executor(params: &[&str], seed: u64) -> ChatExecutor {
    chat_executor(
        &config("", params),
        &BTreeMap::new(),
        StdRng::seed_from_u64(seed),
    )
    .unwrap()
}

fn tally(chat: &ChatExecutor, count: usize) -> Vec<usize> {
    let mut picked = vec![0_usize; chat.deployments.len()];
    for _ in 0..count {
        let deployment = pick_deployment(chat, "g").unwrap();
        let index = chat
            .deployments
            .iter()
            .position(|candidate| candidate.model_id == deployment.model_id)
            .unwrap();
        picked[index] = picked[index].saturating_add(1);
    }
    picked
}

#[test]
fn picks_every_deployment_and_repeats_a_seeded_sequence() {
    let params = ["a\n", "b\n", "c\n"];
    let sequence = |seed| {
        let chat = executor(&params, seed);
        iter::repeat_with(|| pick_deployment(&chat, "g").unwrap().litellm_model.clone())
            .take(40)
            .collect::<Vec<_>>()
    };
    let first = sequence(7);
    assert_eq!(first, sequence(7));
    let picked: BTreeSet<&str> = first.iter().map(String::as_str).collect();
    assert_eq!(picked, BTreeSet::from(["openai/a", "openai/b", "openai/c"]));
}

#[test]
fn a_model_group_nobody_serves_is_an_unknown_model() {
    let chat = executor(&["a\n"], 7);
    let error = pick_deployment(&chat, "missing").unwrap_err();
    assert!(matches!(error, ChatError::UnknownModel { model } if model == "missing"));
}

#[rstest]
#[case::weight(["a\n      weight: 1\n", "b\n      weight: 3\n"], 2300..=2700, 7300..=7700)]
#[case::rpm(["a\n      rpm: 3\n", "b\n      rpm: 1\n"], 7300..=7700, 2300..=2700)]
#[case::tpm(["a\n      tpm: 1\n", "b\n      tpm: 3\n"], 2300..=2700, 7300..=7700)]
fn each_metric_splits_traffic_by_its_share(
    #[case] params: [&str; 2],
    #[case] first: RangeInclusive<usize>,
    #[case] second: RangeInclusive<usize>,
) {
    let picked = tally(&executor(&params, 42), 10_000);
    assert!(first.contains(&picked[0]), "{picked:?}");
    assert!(second.contains(&picked[1]), "{picked:?}");
}

#[test]
fn weight_beats_rpm_and_a_zero_weight_is_never_picked() {
    let chat = executor(
        &[
            "a\n      weight: 0\n      rpm: 900\n",
            "b\n      weight: 1\n      rpm: 1\n",
        ],
        42,
    );
    assert_eq!(tally(&chat, 2_000), vec![0, 2_000]);
}

#[test]
fn rpm_beats_tpm() {
    let chat = executor(
        &["a\n      tpm: 900\n", "b\n      rpm: 1\n      tpm: 1\n"],
        42,
    );
    assert_eq!(tally(&chat, 2_000), vec![0, 2_000]);
}

#[rstest]
#[case::simple_shuffle("router_settings:\n  routing_strategy: simple-shuffle\n")]
#[case::absent("")]
fn simple_shuffle_and_no_strategy_both_serve(#[case] router_settings: &str) {
    let config = config(router_settings, &["a\n"]);
    chat_executor(&config, &BTreeMap::new(), StdRng::seed_from_u64(1)).unwrap();
}

#[rstest]
#[case::least_busy("least-busy")]
#[case::usage_based("usage-based-routing")]
#[case::latency_based("latency-based-routing")]
#[case::cost_based("cost-based-routing")]
fn any_other_strategy_is_refused_by_name(#[case] strategy: &str) {
    let config = config(
        &format!("router_settings:\n  routing_strategy: {strategy}\n"),
        &["a\n"],
    );
    let error = chat_executor(&config, &BTreeMap::new(), StdRng::seed_from_u64(1))
        .err()
        .unwrap();
    assert!(matches!(error, ChatSetupError::RoutingStrategy { .. }));
    assert!(error.to_string().contains("routing_strategy"), "{error}");
}
