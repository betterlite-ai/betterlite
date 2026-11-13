use rstest::rstest;

use crate::finops::vantage::tests::support::{
    FILENAME, FakeSender, HEADER, csv, deliver, expected, rows, uploads,
};

#[rstest]
#[case::one_row_over_the_limit(10_001, &[(".part0", 0, 10_000), (".part1", 10_000, 10_001)])]
#[case::two_full_batches(20_000, &[(".part0", 0, 10_000), (".part1", 10_000, 20_000)])]
#[case::a_partial_third_batch(25_000, &[(".part0", 0, 10_000), (".part1", 10_000, 20_000), (".part2", 20_000, 25_000)])]
#[tokio::test]
async fn rows_over_ten_thousand_split_into_numbered_parts(
    #[case] count: usize,
    #[case] ranges: &[(&str, usize, usize)],
) {
    let lines = rows(count, 8);
    let (result, posted) = deliver(FakeSender::default(), &csv(HEADER, &lines)).await;
    result.unwrap();
    assert_eq!(uploads(&posted), expected(ranges, &lines));
}

#[tokio::test]
async fn blank_lines_are_dropped_from_batches() {
    let lines = rows(10_001, 8);
    let mut with_blanks = lines.clone();
    with_blanks.insert(5, String::new());
    with_blanks.insert(9_000, "  ".to_owned());
    let (result, posted) = deliver(FakeSender::default(), &csv(HEADER, &with_blanks)).await;
    result.unwrap();
    assert_eq!(
        uploads(&posted),
        expected(&[(".part0", 0, 10_000), (".part1", 10_000, 10_001)], &lines)
    );
}

#[tokio::test]
async fn a_record_spanning_lines_is_never_split_across_batches() {
    let mut lines = rows(10_001, 8);
    lines[9_999] = "\"first\n\nthird\"".to_owned();
    let (result, posted) = deliver(FakeSender::default(), &csv(HEADER, &lines)).await;
    result.unwrap();
    assert_eq!(
        uploads(&posted),
        expected(&[(".part0", 0, 10_000), (".part1", 10_000, 10_001)], &lines)
    );
}

#[tokio::test]
async fn filtered_batches_carry_crlf_on_the_header_and_every_row() {
    let body: Vec<String> = (0_usize..10_001)
        .map(|index| format!("A{index},junk"))
        .collect();
    let content = csv("ChargeCategory,Junk", &body);
    let (result, posted) = deliver(FakeSender::default(), &content).await;
    result.unwrap();
    let names: Vec<&str> = posted.iter().map(|post| post.filename.as_str()).collect();
    assert_eq!(
        names,
        [format!("{FILENAME}.part0"), format!("{FILENAME}.part1")]
    );
    let first = String::from_utf8(posted[0].data.clone()).unwrap();
    assert!(
        first.starts_with("ChargeCategory\r\nA0\r\nA1\r\n"),
        "{first}"
    );
    assert!(first.ends_with("A9999\r\n"), "{first}");
    assert_eq!(posted[1].data, b"ChargeCategory\r\nA10000\r\n");
    assert!(!first.replace("\r\n", "").contains(['\r', '\n']));
}
