use rstest::rstest;

use crate::finops::vantage::tests::support::{
    FILENAME, FakeSender, HEADER, SIZE_LIMIT, csv, deliver, rows,
};

fn whole(content: &[u8]) -> Vec<(String, Vec<u8>)> {
    vec![(FILENAME.to_owned(), content.to_vec())]
}

fn multiline_record_content() -> Vec<u8> {
    let mut lines = rows(9_999, 8);
    lines.push("\"first\n\nthird\"".to_owned());
    csv(HEADER, &lines)
}

fn size_limit_row() -> Vec<u8> {
    let row = "x".repeat(SIZE_LIMIT.saturating_sub(HEADER.len()).saturating_sub(2));
    csv(HEADER, &[row])
}

#[rstest]
#[case::empty_content_uploads_nothing(Vec::new(), Vec::new())]
#[case::small_lf_content_goes_up_as_it_is(
    b"ChargeCategory,BilledCost\nA,1\n".to_vec(),
    whole(b"ChargeCategory,BilledCost\nA,1\n")
)]
#[case::unsupported_columns_are_dropped_before_upload(
    b"ChargeCategory,ProviderName,BilledCost\nUsage,x,1\n".to_vec(),
    whole(b"ChargeCategory,BilledCost\r\nUsage,1\r\n")
)]
#[case::filtered_bytes_end_every_record_in_crlf(
    b"ChargeCategory,BilledCost,ServiceName,Junk\nUSA,100.00,Compute,z\n".to_vec(),
    whole(b"ChargeCategory,BilledCost,ServiceName\r\nUSA,100.00,Compute\r\n")
)]
#[case::blank_lines_stay_in_a_single_upload(
    b"ServiceName\nA\n\nB\n".to_vec(),
    whole(b"ServiceName\nA\n\nB\n")
)]
#[case::missing_trailing_newline_is_kept(b"ServiceName\nA".to_vec(), whole(b"ServiceName\nA"))]
#[case::exactly_the_row_limit_is_not_batched(
    csv(HEADER, &rows(10_000, 8)),
    whole(&csv(HEADER, &rows(10_000, 8)))
)]
#[case::a_record_spanning_lines_counts_once_against_the_row_limit(
    multiline_record_content(),
    whole(&multiline_record_content())
)]
#[case::exactly_the_size_limit_is_not_batched(size_limit_row(), whole(&size_limit_row()))]
#[tokio::test]
async fn content_within_both_limits_uploads_in_one_piece(
    #[case] content: Vec<u8>,
    #[case] expected: Vec<(String, Vec<u8>)>,
) {
    let (result, posted) = deliver(FakeSender::default(), &content).await;
    result.unwrap();
    let uploads: Vec<(String, Vec<u8>)> = posted
        .into_iter()
        .map(|post| (post.filename, post.data))
        .collect();
    assert_eq!(uploads, expected);
}
