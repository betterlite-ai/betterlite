#![cfg(test)]

use std::iter;

use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;

use super::error::ShuffleError;
use super::{Candidate, pick};

const SEED: u64 = 42;

fn candidate(weight: u32, rpm: u32, tpm: u32) -> Candidate {
    Candidate { weight, rpm, tpm }
}

fn picks(candidates: &[Candidate], seed: u64, count: usize) -> Vec<usize> {
    let mut random = StdRng::seed_from_u64(seed);
    iter::repeat_with(|| pick(candidates, &mut random).unwrap())
        .take(count)
        .collect()
}

fn counts(candidates: &[Candidate], count: usize) -> Vec<usize> {
    let mut tally = vec![0_usize; candidates.len()];
    for index in picks(candidates, SEED, count) {
        tally[index] = tally[index].saturating_add(1);
    }
    tally
}

#[rstest]
#[case::weight_beats_rpm_and_tpm(vec![candidate(0, 100, 100), candidate(1, 0, 0)], 1)]
#[case::weight_beats_rpm_even_when_rpm_is_larger(vec![candidate(1, 0, 0), candidate(0, 1000, 0)], 0)]
#[case::rpm_beats_tpm(vec![candidate(0, 0, 100), candidate(0, 1, 0)], 1)]
#[case::tpm_decides_when_alone(vec![candidate(0, 0, 0), candidate(0, 0, 5)], 1)]
fn the_first_metric_with_a_positive_value_decides(
    #[case] candidates: Vec<Candidate>,
    #[case] only: usize,
) {
    assert!(
        picks(&candidates, SEED, 200)
            .iter()
            .all(|index| *index == only)
    );
}

#[test]
fn with_no_metric_set_every_deployment_is_picked_about_equally() {
    let tally = counts(&[candidate(0, 0, 0); 4], 10_000);
    assert!(
        tally.iter().all(|count| (2300..=2700).contains(count)),
        "{tally:?}"
    );
}

#[test]
fn weights_one_and_three_split_traffic_a_quarter_to_three_quarters() {
    let tally = counts(&[candidate(1, 0, 0), candidate(3, 0, 0)], 10_000);
    assert!((2300..=2700).contains(&tally[0]), "{tally:?}");
    assert!((7300..=7700).contains(&tally[1]), "{tally:?}");
}

#[test]
fn rpm_values_split_traffic_by_their_share() {
    let tally = counts(&[candidate(0, 1, 9), candidate(0, 3, 0)], 10_000);
    assert!((2300..=2700).contains(&tally[0]), "{tally:?}");
}

#[test]
fn tpm_values_split_traffic_by_their_share() {
    let tally = counts(&[candidate(0, 0, 3), candidate(0, 0, 1)], 10_000);
    assert!((7300..=7700).contains(&tally[0]), "{tally:?}");
}

#[test]
fn a_zero_weight_is_never_picked_while_another_deployment_has_a_positive_one() {
    let candidates = [
        candidate(0, 500, 500),
        candidate(2, 0, 0),
        candidate(0, 500, 500),
        candidate(5, 0, 0),
    ];
    let picked = picks(&candidates, SEED, 5_000);
    assert!(picked.iter().all(|index| *index == 1 || *index == 3));
    assert!(picked.contains(&1) && picked.contains(&3));
}

#[test]
fn the_same_seed_gives_the_same_picks_and_another_seed_gives_others() {
    let candidates = [candidate(1, 0, 0), candidate(3, 0, 0), candidate(2, 0, 0)];
    let first = picks(&candidates, SEED, 100);
    assert_eq!(first, picks(&candidates, SEED, 100));
    assert_ne!(first, picks(&candidates, SEED + 1, 100));
}

#[test]
fn a_single_deployment_is_always_picked() {
    assert_eq!(picks(&[candidate(0, 0, 0)], SEED, 20), vec![0; 20]);
}

#[test]
fn no_deployments_is_an_error() {
    let mut random = StdRng::seed_from_u64(SEED);
    assert!(matches!(
        pick(&[], &mut random),
        Err(ShuffleError::NoCandidates)
    ));
}
