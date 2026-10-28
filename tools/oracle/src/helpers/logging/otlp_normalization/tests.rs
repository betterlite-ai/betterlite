#![cfg(test)]

use std::path::PathBuf;
use std::process::Command;

use rstest::rstest;

fn assert_matches_litellm(case: &PathBuf) {
    let output = Command::new(env!("CARGO_BIN_EXE_betterlite-oracle"))
        .arg(case)
        .output()
        .unwrap();
    let report = String::from_utf8(output.stderr).unwrap();
    assert!(output.status.success(), "{}: {report}", case.display());
}

#[rstest]
fn otlp_truncate_helper(
    #[files("cases/helpers/otlp-normalization/truncate-*.yaml")] case: PathBuf,
) {
    assert_matches_litellm(&case);
}

#[rstest]
fn otlp_elision_helper(#[files("cases/helpers/otlp-normalization/payload-*.yaml")] case: PathBuf) {
    assert_matches_litellm(&case);
}

#[rstest]
fn otlp_content_text_helper(
    #[files("cases/helpers/otlp-normalization/content-text-*.yaml")] case: PathBuf,
) {
    assert_matches_litellm(&case);
}

#[rstest]
fn otlp_ui_content_helper(
    #[files("cases/helpers/otlp-normalization/ui-content-*.yaml")]
    #[files("cases/helpers/otlp-normalization/role-mapping-*.yaml")]
    case: PathBuf,
) {
    assert_matches_litellm(&case);
}

#[rstest]
fn otlp_response_encoding_helper(
    #[files("cases/helpers/otlp-normalization/response-*.yaml")] case: PathBuf,
) {
    assert_matches_litellm(&case);
}
