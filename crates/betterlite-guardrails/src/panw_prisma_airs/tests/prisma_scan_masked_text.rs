use rstest::rstest;

use crate::panw_prisma_airs::verdict::{Action, ScanVerdict};
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

#[rstest]
#[case::prompt_string(
    r#"{"action":"allow","category":"benign","prompt_masked_data":"redacted"}"#,
    false
)]
#[case::response_list(
    r#"{"action":"allow","category":"benign","response_masked_data":["redacted"]}"#,
    true
)]
fn a_masked_field_that_is_not_an_object_keeps_the_verdict(
    #[case] body: &str,
    #[case] is_response: bool,
) {
    let verdict = ScanVerdict::from_body(body.as_bytes()).expect("the verdict survives");
    assert_eq!(verdict.action, Action::Allow);
    assert_eq!(verdict.category.as_deref(), Some("benign"));
    assert_eq!(get_masked_text(&verdict.masks, is_response), None);
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
