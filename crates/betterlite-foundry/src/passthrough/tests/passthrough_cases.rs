mod auth_cases;
mod ocr_cases;
mod relay_cases;
mod stream_cases;
mod url_cases;

use rstest::rstest;

pub(super) const RES: &str = "https://res.services.ai.azure.com";
pub(super) const OCR_BODY: &str = r##"{"pages":[{"index":0,"markdown":"# page"}],"model":"mistral-ocr-latest","usage_info":{"pages_processed":2,"credits":1.5}}"##;

#[rstest]
#[case::foundry_root_path(0)]
#[case::repeated_native_prefix(1)]
#[case::query_api_version_precedence(2)]
#[case::ocr_usage_log(3)]
#[case::stream_collected_log(4)]
#[case::auth_selection(5)]
#[case::foundry_relay_shape_literals(6)]
fn passthrough_cases(#[case] case: u8) {
    match case {
        0 => url_cases::foundry_root_path(),
        1 => url_cases::repeated_native_prefix(),
        2 => url_cases::query_api_version_precedence(),
        3 => ocr_cases::ocr_usage_log(),
        4 => stream_cases::stream_collected_log(),
        5 => auth_cases::auth_selection(),
        _ => relay_cases::foundry_relay_shape_literals(),
    }
}
