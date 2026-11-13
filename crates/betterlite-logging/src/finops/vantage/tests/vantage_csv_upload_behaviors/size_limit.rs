use rstest::rstest;

use crate::finops::vantage::tests::support::{
    FakeSender, HEADER, SIZE_LIMIT, csv, deliver, expected, rows, uploads,
};

fn wide(widths: &[usize]) -> Vec<String> {
    widths.iter().map(|width| "x".repeat(*width)).collect()
}

// the header line costs 12 bytes, so a row of 2_097_139 characters fills a part exactly
const FILLS_A_PART: usize = SIZE_LIMIT - 13;
const HALF_LESS_SIX: usize = 1024 * 1024 - 6;
const HALF_LESS_SEVEN: usize = 1024 * 1024 - 7;

#[rstest]
#[case::three_wide_rows(&[700_000, 700_000, 700_000], &[(".part0_0", 0, 2), (".part0_1", 2, 3)])]
#[case::one_byte_over_the_limit(&[HALF_LESS_SIX, HALF_LESS_SEVEN], &[(".part0_0", 0, 1), (".part0_1", 1, 2)])]
#[case::a_part_filled_to_the_byte_stays_whole(&[HALF_LESS_SEVEN, HALF_LESS_SEVEN, 1], &[(".part0_0", 0, 2), (".part0_1", 2, 3)])]
#[case::a_row_that_fills_a_part_is_not_skipped(&[1, FILLS_A_PART, 1], &[(".part0_0", 0, 1), (".part0_1", 1, 2), (".part0_2", 2, 3)])]
#[tokio::test]
async fn parts_over_two_mib_split_by_size_with_batch_and_part_indices(
    #[case] widths: &[usize],
    #[case] ranges: &[(&str, usize, usize)],
) {
    let lines = wide(widths);
    let (result, posted) = deliver(FakeSender::default(), &csv(HEADER, &lines)).await;
    result.unwrap();
    assert_eq!(uploads(&posted), expected(ranges, &lines));
    assert!(posted.iter().all(|post| post.data.len() <= SIZE_LIMIT));
}

#[rstest]
#[case::between_small_rows(&[1, FILLS_A_PART + 1, 1], &[1, 1])]
#[case::alone(&[FILLS_A_PART + 1], &[])]
#[tokio::test]
async fn a_row_too_big_even_with_only_the_header_is_skipped(
    #[case] widths: &[usize],
    #[case] kept: &[usize],
) {
    let lines = wide(widths);
    let (result, posted) = deliver(FakeSender::default(), &csv(HEADER, &lines)).await;
    result.unwrap();
    let survivors = wide(kept);
    let expected_uploads = if survivors.is_empty() {
        Vec::new()
    } else {
        expected(&[(".part0_0", 0, 2)], &survivors)
    };
    assert_eq!(uploads(&posted), expected_uploads);
}

#[tokio::test]
async fn row_batches_over_the_size_limit_get_sub_indices_and_the_next_batch_keeps_its_own() {
    let lines = rows(10_001, 250);
    let (result, posted) = deliver(FakeSender::default(), &csv(HEADER, &lines)).await;
    result.unwrap();
    let ranges = [
        (".part0_0", 0, 8_355),
        (".part0_1", 8_355, 10_000),
        (".part1", 10_000, 10_001),
    ];
    assert_eq!(uploads(&posted), expected(&ranges, &lines));
    assert!(posted.iter().all(|post| post.data.len() <= SIZE_LIMIT));
}
