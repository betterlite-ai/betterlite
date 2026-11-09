use rstest::rstest;

use super::cases::{Expected, check};

const BAD_DURATION: &str = "Invalid Marengo 3.0 parameters: segmentation.TwelveLabsMarengo3FixedSegmentation.fixed.durationSec: Input should be a valid integer; segmentation.TwelveLabsMarengo3DynamicSegmentation.method: Input should be 'dynamic'; segmentation.TwelveLabsMarengo3DynamicSegmentation.dynamic: Field required";

fn fixed_duration_call(duration: &str) -> String {
    format!(
        r#"{{"input": "AAA", "params": {{"inputType": "video", "segmentation": {{"method": "fixed", "fixed": {{"durationSec": {duration}}}}}}}}}"#
    )
}

fn fixed_duration_body(duration: &str) -> String {
    format!(
        r#"{{"inputType": "video", "video": {{"mediaSource": {{"base64String": "AAA"}}, "segmentation": {{"method": "fixed", "fixed": {{"durationSec": {duration}}}}}}}}}"#
    )
}

#[rstest]
#[case::at_the_limit("1".repeat(4300), true)]
#[case::over_the_limit("1".repeat(4301), false)]
#[case::leading_zeros_do_not_count(format!("{}1", "0".repeat(5000)), true)]
#[case::negative_at_the_limit(format!("-{}", "1".repeat(4299)), true)]
#[case::negative_over_the_limit(format!("-{}", "1".repeat(4300)), false)]
#[case::plus_sign_does_not_count(format!("+{}", "1".repeat(4300)), true)]
fn text_length_limit(#[case] digits: String, #[case] accepted: bool) {
    let call = fixed_duration_call(&format!("\"{digits}\""));
    if accepted {
        let significant = digits.trim_start_matches(['+', '0']);
        check(&call, &Expected::Built(&fixed_duration_body(significant)));
    } else {
        check(&call, &Expected::Rejected(BAD_DURATION));
    }
}

#[rstest]
#[case::huge_whole_duration_number(
    r#"{"input": "AAA", "params": {"inputType": "video", "segmentation": {"method": "fixed", "fixed": {"durationSec": 100000000000000000000}}}}"#,
    Expected::Built(
        r#"{"inputType": "video", "video": {"mediaSource": {"base64String": "AAA"}, "segmentation": {"method": "fixed", "fixed": {"durationSec": 100000000000000000000}}}}"#
    )
)]
#[case::huge_whole_duration_text(
    r#"{"input": "AAA", "params": {"inputType": "video", "segmentation": {"method": "fixed", "fixed": {"durationSec": " 100000000000000000000 "}}}}"#,
    Expected::Built(
        r#"{"inputType": "video", "video": {"mediaSource": {"base64String": "AAA"}, "segmentation": {"method": "fixed", "fixed": {"durationSec": 100000000000000000000}}}}"#
    )
)]
#[case::max_u64_duration_text(
    r#"{"input": "AAA", "params": {"inputType": "audio", "segmentation": {"method": "dynamic", "dynamic": {"minDurationSec": "18446744073709551615"}}}}"#,
    Expected::Built(
        r#"{"inputType": "audio", "audio": {"mediaSource": {"base64String": "AAA"}, "segmentation": {"method": "dynamic", "dynamic": {"minDurationSec": 18446744073709551615}}}}"#
    )
)]
#[case::signed_and_padded_duration_text(
    r#"{"input": "AAA", "params": {"inputType": "audio", "segmentation": {"method": "dynamic", "dynamic": {"minDurationSec": "+007"}}}}"#,
    Expected::Built(
        r#"{"inputType": "audio", "audio": {"mediaSource": {"base64String": "AAA"}, "segmentation": {"method": "dynamic", "dynamic": {"minDurationSec": 7}}}}"#
    )
)]
#[case::huge_whole_duration_leading_zeros(
    r#"{"input": "AAA", "params": {"inputType": "video", "segmentation": {"method": "fixed", "fixed": {"durationSec": "0100000000000000000000"}}}}"#,
    Expected::Built(
        r#"{"inputType": "video", "video": {"mediaSource": {"base64String": "AAA"}, "segmentation": {"method": "fixed", "fixed": {"durationSec": 100000000000000000000}}}}"#
    )
)]
#[case::huge_whole_duration_signed_leading_zeros(
    r#"{"input": "AAA", "params": {"inputType": "video", "segmentation": {"method": "fixed", "fixed": {"durationSec": "+0100000000000000000000"}}}}"#,
    Expected::Built(
        r#"{"inputType": "video", "video": {"mediaSource": {"base64String": "AAA"}, "segmentation": {"method": "fixed", "fixed": {"durationSec": 100000000000000000000}}}}"#
    )
)]
#[case::huge_whole_duration_zero_fraction(
    r#"{"input": "AAA", "params": {"inputType": "video", "segmentation": {"method": "fixed", "fixed": {"durationSec": "100000000000000000000.0"}}}}"#,
    Expected::Built(
        r#"{"inputType": "video", "video": {"mediaSource": {"base64String": "AAA"}, "segmentation": {"method": "fixed", "fixed": {"durationSec": 100000000000000000000}}}}"#
    )
)]
#[case::negative_huge_whole_duration_text(
    r#"{"input": "AAA", "params": {"inputType": "video", "segmentation": {"method": "fixed", "fixed": {"durationSec": "-0100000000000000000000.00"}}}}"#,
    Expected::Built(
        r#"{"inputType": "video", "video": {"mediaSource": {"base64String": "AAA"}, "segmentation": {"method": "fixed", "fixed": {"durationSec": -100000000000000000000}}}}"#
    )
)]
#[case::negative_zero_duration_number(
    r#"{"input": "AAA", "params": {"inputType": "video", "segmentation": {"method": "fixed", "fixed": {"durationSec": -0}}}}"#,
    Expected::Built(
        r#"{"inputType": "video", "video": {"mediaSource": {"base64String": "AAA"}, "segmentation": {"method": "fixed", "fixed": {"durationSec": 0}}}}"#
    )
)]
#[case::negative_zero_duration_text(
    r#"{"input": "AAA", "params": {"inputType": "video", "segmentation": {"method": "fixed", "fixed": {"durationSec": "-0"}}}}"#,
    Expected::Built(
        r#"{"inputType": "video", "video": {"mediaSource": {"base64String": "AAA"}, "segmentation": {"method": "fixed", "fixed": {"durationSec": 0}}}}"#
    )
)]
#[case::negative_zero_fraction_duration_text(
    r#"{"input": "AAA", "params": {"inputType": "audio", "segmentation": {"method": "dynamic", "dynamic": {"minDurationSec": "-0.0"}}}}"#,
    Expected::Built(
        r#"{"inputType": "audio", "audio": {"mediaSource": {"base64String": "AAA"}, "segmentation": {"method": "dynamic", "dynamic": {"minDurationSec": 0}}}}"#
    )
)]
fn whole_digit_durations(#[case] call: &str, #[case] expected: Expected<'_>) {
    check(call, &expected);
}

#[rstest]
#[case::float_duration_beyond_integer_range(
    r#"{"segmentation": {"method": "fixed", "fixed": {"durationSec": 1e19}}}"#,
    "segmentation.TwelveLabsMarengo3FixedSegmentation.fixed.durationSec: Input should be a valid integer; segmentation.TwelveLabsMarengo3DynamicSegmentation.method: Input should be 'dynamic'; segmentation.TwelveLabsMarengo3DynamicSegmentation.dynamic: Field required"
)]
#[case::double_sign_duration_text(
    r#"{"segmentation": {"method": "fixed", "fixed": {"durationSec": "+-5"}}}"#,
    "segmentation.TwelveLabsMarengo3FixedSegmentation.fixed.durationSec: Input should be a valid integer; segmentation.TwelveLabsMarengo3DynamicSegmentation.method: Input should be 'dynamic'; segmentation.TwelveLabsMarengo3DynamicSegmentation.dynamic: Field required"
)]
#[case::dotted_duration_text_without_fraction(
    r#"{"segmentation": {"method": "fixed", "fixed": {"durationSec": "4."}}}"#,
    "segmentation.TwelveLabsMarengo3FixedSegmentation.fixed.durationSec: Input should be a valid integer; segmentation.TwelveLabsMarengo3DynamicSegmentation.method: Input should be 'dynamic'; segmentation.TwelveLabsMarengo3DynamicSegmentation.dynamic: Field required"
)]
fn invalid_durations(#[case] params: &str, #[case] details: &str) {
    let call = format!(r#"{{"input": "x", "params": {params}}}"#);
    let message = format!("Invalid Marengo 3.0 parameters: {details}");
    check(&call, &Expected::Rejected(&message));
}
