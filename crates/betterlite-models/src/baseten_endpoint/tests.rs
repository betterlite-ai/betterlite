#![cfg(test)]

use rstest::rstest;

use super::{INFERENCE_API_BASE, get_api_base_for_model, is_dedicated_deployment};

#[rstest]
#[case::plain_id("abcd1234", true)]
#[case::prefixed_id("baseten/abcd1234", true)]
#[case::uppercase_id("baseten/ABCD1234", true)]
#[case::all_digits("12345678", true)]
#[case::repeated_prefix("baseten/baseten/abcd1234", true)]
#[case::seven_characters("baseten/abcd123", false)]
#[case::nine_characters("baseten/abcd12345", false)]
#[case::hyphen("baseten/abcd-123", false)]
#[case::underscore("baseten/abcd_123", false)]
#[case::empty_id("baseten/", false)]
#[case::empty_model("", false)]
#[case::trailing_slash("baseten/abcd1234/", false)]
#[case::non_ascii("baseten/\u{430}\u{431}\u{432}\u{433}1234", false)]
#[case::seven_digits("1234567", false)]
#[case::trailing_newline("baseten/abcd1234\n", false)]
fn dedicated_deployment_is_eight_ascii_alphanumerics(#[case] model: &str, #[case] dedicated: bool) {
    assert_eq!(is_dedicated_deployment(model), dedicated);
}

#[rstest]
#[case::plain_id("abcd1234")]
#[case::prefixed_id("baseten/abcd1234")]
#[case::repeated_prefix("baseten/baseten/abcd1234")]
#[case::uppercase_id("baseten/ABCD1234")]
fn dedicated_deployment_gets_the_model_host(#[case] model: &str) {
    let deployment = model.replace("baseten/", "");
    assert_eq!(
        get_api_base_for_model(model),
        format!("https://model-{deployment}.api.baseten.co/environments/production/sync/v1")
    );
}

#[rstest]
#[case::shorter_id("baseten/abcd123")]
#[case::longer_id("baseten/abcd12345")]
#[case::hyphenated("baseten/abcd-123")]
#[case::empty("")]
#[case::trailing_newline("baseten/abcd1234\n")]
fn other_models_use_the_inference_host(#[case] model: &str) {
    assert_eq!(get_api_base_for_model(model), INFERENCE_API_BASE);
}

#[test]
fn the_inference_host_is_the_documented_default() {
    assert_eq!(INFERENCE_API_BASE, "https://inference.baseten.co/v1");
}
