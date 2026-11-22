use rstest::rstest;

use super::super::aggregation::{TagAggregationError, tag_spend_body};
use super::support::{daily_tag, decimal};

#[test]
fn sums_across_dates_and_ranks_by_spend() {
    let rows = [
        daily_tag(Some("a"), "1.5", 1),
        daily_tag(Some("b"), "2.25", 1),
        daily_tag(Some("a"), "2.25", 2),
    ];
    let body = tag_spend_body(&rows).unwrap();
    assert_eq!(body.spend_per_tag.len(), 2);
    assert_eq!(body.spend_per_tag[0].name.as_deref(), Some("a"));
    assert_eq!(body.spend_per_tag[0].spend, decimal("3.75"));
    assert_eq!(body.spend_per_tag[0].log_count, 3);
    assert_eq!(body.spend_per_tag[1].name.as_deref(), Some("b"));
    assert_eq!(body.spend_per_tag[1].spend, decimal("2.25"));
    assert_eq!(body.spend_per_tag[1].log_count, 1);
}

#[test]
fn keeps_the_source_order_of_equal_spends() {
    let rows = [
        daily_tag(Some("second"), "1", 1),
        daily_tag(Some("first"), "1", 1),
        daily_tag(Some("third"), "2", 1),
    ];
    let body = tag_spend_body(&rows).unwrap();
    let names: Vec<Option<&str>> = body
        .spend_per_tag
        .iter()
        .map(|row| row.name.as_deref())
        .collect();
    assert_eq!(names, vec![Some("third"), Some("second"), Some("first")]);
}

#[test]
fn treats_a_null_tag_as_one_bucket() {
    let rows = [
        daily_tag(None, "1", 1),
        daily_tag(Some("a"), "4", 1),
        daily_tag(None, "2", 2),
    ];
    let body = tag_spend_body(&rows).unwrap();
    assert_eq!(body.spend_per_tag[0].name, None);
    assert_eq!(body.spend_per_tag[0].spend, decimal("3"));
    assert_eq!(body.spend_per_tag[0].log_count, 3);
    assert_eq!(body.spend_per_tag[1].name.as_deref(), Some("a"));
}

#[test]
fn returns_every_tag_past_ten() {
    let names: Vec<String> = (0_i32..12).map(|index| format!("tag-{index}")).collect();
    let rows: Vec<_> = names
        .iter()
        .zip(1_i64..)
        .map(|(name, spend)| daily_tag(Some(name.as_str()), &spend.to_string(), 1))
        .collect();
    let body = tag_spend_body(&rows).unwrap();
    assert_eq!(body.spend_per_tag.len(), 12);
    assert_eq!(body.spend_per_tag[0].name.as_deref(), Some("tag-11"));
    assert_eq!(body.spend_per_tag[11].name.as_deref(), Some("tag-0"));
}

#[rstest]
#[case::rounds_down_at_the_even_tie("0.00005", "0")]
#[case::rounds_up_at_the_odd_tie("0.00015", "0.0002")]
#[case::keeps_a_shorter_total("1.5", "1.5")]
#[case::rounds_the_fifth_digit_up("1.23456", "1.2346")]
#[case::rounds_the_fifth_digit_down("1.23454", "1.2345")]
#[case::keeps_an_even_digit_tie("0.00025", "0.0002")]
#[case::raises_an_odd_digit_tie("0.00035", "0.0004")]
fn rounds_the_aggregated_spend(#[case] stored: &str, #[case] expected: &str) {
    let rows = [daily_tag(Some("a"), stored, 1)];
    let body = tag_spend_body(&rows).unwrap();
    assert_eq!(body.spend_per_tag[0].spend, decimal(expected));
}

#[test]
fn rejects_an_amount_sum_that_overflows() {
    let rows = [
        daily_tag(Some("a"), "79228162514264337593543950335", 1),
        daily_tag(Some("a"), "1", 1),
    ];
    assert!(matches!(
        tag_spend_body(&rows),
        Err(TagAggregationError::AmountOverflow)
    ));
}

#[test]
fn rejects_a_count_sum_that_overflows() {
    let rows = [
        daily_tag(Some("a"), "1", i64::MAX),
        daily_tag(Some("a"), "1", 1),
    ];
    assert!(matches!(
        tag_spend_body(&rows),
        Err(TagAggregationError::CountOverflow)
    ));
}

#[test]
fn aggregates_an_empty_result_to_no_rows() {
    assert!(tag_spend_body(&[]).unwrap().spend_per_tag.is_empty());
}
