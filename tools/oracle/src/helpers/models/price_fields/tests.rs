#![cfg(test)]

use super::{Input, Op};
use crate::helpers::Helper as _;
use rstest::rstest;

#[rstest]
#[case::direct(r#"{"input_cost_per_token":0.1000}"#, "/input_cost_per_token", "0.1")]
#[case::source(
    r#"{"source":"{\"input_cost_per_token\":0.0000001234567890123456789}"}"#,
    "/input_cost_per_token",
    "0.0000001234567890123456789"
)]
#[case::tier(
    r#"{"tiered_pricing":[{"range":[0.0,2.0]}]}"#,
    "/tiered_pricing/0/range/1",
    "2"
)]
#[case::hours(
    r#"{"off_peak_pricing":{"hours_utc":"00"}}"#,
    "/off_peak_pricing/hours_utc",
    "00"
)]
fn canonical_projection(#[case] source: &str, #[case] path: &str, #[case] expected: &str) {
    let input: Input = serde_json::from_str(source).unwrap();
    let output = Op::run(&input).unwrap();
    assert_eq!(output.pointer(path).unwrap().as_str(), Some(expected));
}

#[rstest]
#[case::invalid_source(r#"{"source":"invalid"}"#)]
#[case::null_price(r#"{"source":"{\"input_cost_per_token\":null}"}"#)]
fn rejects_invalid_prices(#[case] source: &str) {
    let input: Input = serde_json::from_str(source).unwrap();
    let error = Op::run(&input).unwrap_err();
    assert_eq!(error.class, "PriceFieldError");
}

#[rstest]
#[case::null(r#"{"input_cost_per_token":null}"#)]
#[case::underflow(r#"{"input_cost_per_token":1e-29}"#)]
#[case::string(r#"{"input_cost_per_token":"0.1"}"#)]
fn rejects_invalid_typed_input(#[case] source: &str) {
    let result = serde_json::from_str::<Input>(source);
    assert!(result.is_err());
}

#[rstest]
#[case::float(r#"{'input_cost_per_token': 0.1}"#, "type(None)")]
#[case::overprecision(
    r#"{'source': '{"input_cost_per_token":0.123456789012345678901234567891}'}"#,
    "Inexact"
)]
fn python_rejects_lossy_prices(#[case] input: &str, #[case] cause: &str) {
    let helpers = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("helpers");
    let script = format!(
        "from decimal import Inexact\nfrom price_fields import PriceFieldError, run\ntry:\n    run({input})\nexcept PriceFieldError as error:\n    assert type(error.__cause__) is {cause}\nelse:\n    raise AssertionError('lossy price accepted')\n"
    );
    let output = std::process::Command::new("setpriv")
        .args(["--pdeathsig", "KILL", "--", "python3"])
        .arg("-c")
        .arg(script)
        .current_dir(helpers)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
