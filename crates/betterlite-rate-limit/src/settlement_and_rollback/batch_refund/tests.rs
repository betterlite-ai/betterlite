#![cfg(test)]

use rstest::rstest;

use crate::limiter_records::{CacheCounterValue, CounterRefund};
use crate::settlement_and_rollback::SettlementError;
use crate::settlement_and_rollback::refunds::{
    // ast-grep-ignore: no-long-names
    counter_refunds_from_batch_values as batch_refunds,
};
use crate::settlement_and_rollback::tests::support::{int_value, key_list, text_value};

fn projected(refunds: &[CounterRefund]) -> Vec<(String, String, String, i64)> {
    refunds
        .iter()
        .map(|refund| {
            (
                refund.window_key.clone(),
                refund.counter_key.clone(),
                refund.window_start.clone(),
                refund.increment,
            )
        })
        .collect()
}

#[rstest]
#[case::two_pairs(
    vec![
        "{model:gpt-4}:window",
        "{model:gpt-4}:requests",
        "{user:alice}:window",
        "{user:alice}:requests",
    ],
    vec![
        Some(text_value("100")),
        Some(int_value(4)),
        Some(text_value("200")),
        Some(int_value(5)),
    ],
    vec![
        ("{model:gpt-4}:window", "{model:gpt-4}:requests", "100"),
        ("{user:alice}:window", "{user:alice}:requests", "200"),
    ]
)]
#[case::missing_window_start_is_skipped(
    vec![
        "{model:gpt-4}:window",
        "{model:gpt-4}:requests",
        "{user:alice}:window",
        "{user:alice}:requests",
    ],
    vec![
        None,
        Some(int_value(4)),
        Some(text_value("200")),
        Some(int_value(5)),
    ],
    vec![("{user:alice}:window", "{user:alice}:requests", "200")]
)]
#[case::fewer_values_than_pairs(
    vec![
        "{model:gpt-4}:window",
        "{model:gpt-4}:requests",
        "{user:alice}:window",
        "{user:alice}:requests",
    ],
    vec![Some(text_value("100"))],
    vec![("{model:gpt-4}:window", "{model:gpt-4}:requests", "100")]
)]
#[case::no_pairs(vec![], vec![], vec![])]
fn alternating_keys_build_one_refund_per_pair(
    #[case] keys: Vec<&str>,
    #[case] values: Vec<Option<CacheCounterValue>>,
    #[case] expected: Vec<(&str, &str, &str)>,
) {
    let refunds = batch_refunds(&key_list(&keys), &values).expect("the values pair up");
    assert_eq!(
        projected(&refunds),
        expected
            .iter()
            .map(|(window_key, counter_key, start)| {
                (
                    (*window_key).to_owned(),
                    (*counter_key).to_owned(),
                    (*start).to_owned(),
                    1,
                )
            })
            .collect::<Vec<_>>()
    );
}

#[test]
fn an_odd_key_count_with_enough_values_is_an_error() {
    let keys = key_list(&[
        "{model:gpt-4}:window",
        "{model:gpt-4}:requests",
        "{user:alice}:window",
    ]);
    let values = vec![
        Some(text_value("100")),
        Some(int_value(4)),
        Some(text_value("200")),
    ];
    let error = batch_refunds(&keys, &values).expect_err("the last pair has no counter key");
    assert!(matches!(
        error,
        SettlementError::UnpairedRefundKeys { count: 3 }
    ));
}

#[test]
fn a_float_window_start_is_truncated_like_a_whole_number() {
    let keys = key_list(&["{model:gpt-4}:window", "{model:gpt-4}:requests"]);
    let values = vec![Some(CacheCounterValue::Float(7.9)), Some(int_value(4))];
    let refunds = batch_refunds(&keys, &values).expect("the window start has a whole form");
    assert_eq!(
        refunds.first().map(|refund| refund.window_start.as_str()),
        Some("7")
    );
}

#[test]
fn a_non_numeric_window_start_is_an_error() {
    let keys = key_list(&["{model:gpt-4}:window", "{model:gpt-4}:requests"]);
    let values = vec![Some(text_value("soon")), Some(int_value(4))];
    let error = batch_refunds(&keys, &values).expect_err("the window start is not a number");
    assert!(matches!(error, SettlementError::WindowCounter(_)));
}
