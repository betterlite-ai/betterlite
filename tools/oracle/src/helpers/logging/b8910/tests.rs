#![cfg(test)]

use std::path::Path;

use betterlite_oracle_runner::case::load_helper_cases;
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};
use rstest::rstest;

#[rstest]
#[case::generic_and_langfuse_class("generic-and-langfuse-class.yaml")]
#[case::langfuse_content_class("langfuse-content-class.yaml")]
#[case::gated_and_credentialed_exporters("gated-and-credentialed-exporters.yaml")]
#[case::console_placeholder_not_operator("console-placeholder-not-operator.yaml")]
#[case::empty_exporters_export_nowhere("empty-exporters-export-nowhere.yaml")]
#[case::retain_only_gated_exporters("retain-only-gated-exporters.yaml")]
fn helper_case_matches_litellm(#[case] name: &str) {
    let path = Path::new(ORACLE_DIR)
        .join("cases/helpers/otel-logger-runtime")
        .join(name);
    let cases = load_helper_cases(&[path]).expect("the case loads");
    let Some(case) = cases.first() else {
        panic!("expected a helper case");
    };
    assert!(matches!(
        run_case(&[crate::GROUP], case, false).expect("the case runs"),
        HelperRun::Match
    ));
}
