use rstest::rstest;

use crate::prisma_airs_masking::{MaskedData, ScanResult, get_masked_text};

fn masked(data: Option<&str>) -> MaskedData {
    MaskedData {
        data: data.map(str::to_owned),
    }
}

#[rstest]
#[case::request_side(false, Some("prompt text"))]
#[case::response_side(true, Some("response text"))]
fn each_direction_reads_its_own_masked_data(
    #[case] is_response: bool,
    #[case] expected: Option<&str>,
) {
    let result = ScanResult {
        prompt_masked_data: Some(masked(Some("prompt text"))),
        response_masked_data: Some(masked(Some("response text"))),
    };
    assert_eq!(get_masked_text(&result, is_response), expected);
}

#[rstest]
#[case::no_masked_block(None, None)]
#[case::block_without_data(Some(masked(None)), None)]
#[case::empty_data_is_kept(Some(masked(Some(""))), Some(""))]
fn a_missing_or_empty_masked_text_is_reported_as_it_is(
    #[case] prompt: Option<MaskedData>,
    #[case] expected: Option<&str>,
) {
    let result = ScanResult {
        prompt_masked_data: prompt,
        response_masked_data: None,
    };
    assert_eq!(get_masked_text(&result, false), expected);
    assert_eq!(get_masked_text(&result, true), None);
}

#[test]
fn masked_data_parses_from_a_scan_body_and_null_means_none() {
    let result: ScanResult = serde_json::from_str(
        r#"{"action":"block","prompt_masked_data":{"data":"x"},"response_masked_data":null}"#,
    )
    .expect("scan result parses");
    assert_eq!(get_masked_text(&result, false), Some("x"));
    assert_eq!(get_masked_text(&result, true), None);
}
