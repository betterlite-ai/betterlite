#![cfg(test)]

use rstest::rstest;

use super::{OrderedDeployment, get_order_filtered_deployments};

#[derive(Clone, Debug, Eq, PartialEq)]
struct Deployment {
    id: u8,
    params_order: Option<i64>,
    info_order: Option<i64>,
}

impl OrderedDeployment for Deployment {
    fn litellm_params_order(&self) -> Option<i64> {
        self.params_order
    }

    fn model_info_order(&self) -> Option<i64> {
        self.info_order
    }
}

fn deployment(id: u8, params_order: Option<i64>, info_order: Option<i64>) -> Deployment {
    Deployment {
        id,
        params_order,
        info_order,
    }
}

fn ids(deployments: &[Deployment]) -> Vec<u8> {
    deployments.iter().map(|found| found.id).collect()
}

#[rstest]
#[case::params_wins_over_info(Some(2), Some(1), Some(2))]
#[case::info_when_params_has_none(None, Some(3), Some(3))]
#[case::params_zero_is_an_order(Some(0), Some(5), Some(0))]
#[case::neither(None, None, None)]
fn order_reads_litellm_params_first_then_model_info(
    #[case] params_order: Option<i64>,
    #[case] info_order: Option<i64>,
    #[case] expected: Option<i64>,
) {
    assert_eq!(deployment(1, params_order, info_order).order(), expected);
}

#[rstest]
#[case::smallest_group(
    vec![deployment(1, Some(2), None), deployment(2, Some(1), None), deployment(3, Some(1), None)],
    None,
    vec![2, 3]
)]
#[case::group_from_model_info(
    vec![deployment(1, Some(2), None), deployment(2, None, Some(1))],
    None,
    vec![2]
)]
#[case::zero_beats_positive(
    vec![deployment(1, Some(1), None), deployment(2, Some(0), None)],
    None,
    vec![2]
)]
#[case::negative_is_smallest(
    vec![deployment(1, Some(-1), None), deployment(2, Some(0), None)],
    None,
    vec![1]
)]
#[case::deployments_without_order_drop_out_of_the_smallest_group(
    vec![deployment(1, None, None), deployment(2, Some(4), None)],
    None,
    vec![2]
)]
#[case::no_orders_returns_the_input(
    vec![deployment(1, None, None), deployment(2, None, None)],
    None,
    vec![1, 2]
)]
#[case::empty_input(vec![], None, vec![])]
#[case::target_selects_its_group(
    vec![deployment(1, Some(1), None), deployment(2, Some(2), None), deployment(3, None, Some(2))],
    Some(2),
    vec![2, 3]
)]
#[case::target_without_a_match_gives_nothing(
    vec![deployment(1, Some(1), None), deployment(2, None, None)],
    Some(7),
    vec![]
)]
#[case::target_on_unordered_deployments_gives_nothing(
    vec![deployment(1, None, None)],
    Some(1),
    vec![]
)]
fn order_filter_keeps_one_group(
    #[case] deployments: Vec<Deployment>,
    #[case] target_order: Option<i64>,
    #[case] expected: Vec<u8>,
) {
    let kept = get_order_filtered_deployments(deployments, target_order);
    assert_eq!(ids(&kept), expected);
}
