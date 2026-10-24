#![cfg(test)]

use crate::deviations::mismatch;
use crate::spec::deviations::DeviationLog;
use crate::spec::report::ReportDeviation;

fn deviation(what: &str) -> ReportDeviation {
    ReportDeviation {
        what: what.to_owned(),
        why: "LiteLLM drops the header".to_owned(),
        link: String::new(),
    }
}

#[test]
fn no_deviations_need_no_log() {
    assert!(mismatch(&[], None, "B003").is_empty());
}

#[test]
fn reported_deviations_need_a_log() {
    let problems = mismatch(&[deviation("keeps the header")], None, "B003");
    assert_eq!(
        problems,
        vec!["the report lists 1 deviations; write the same entries to docs/deviations/B003.yaml"]
    );
}

#[test]
fn a_log_matching_the_report_passes_in_any_order() {
    let reported = [deviation("a"), deviation("b")];
    let log = DeviationLog {
        task: "B003".to_owned(),
        deviations: vec![deviation("b"), deviation("a")],
    };
    assert!(mismatch(&reported, Some(&log), "B003").is_empty());
}

#[test]
fn a_log_that_differs_names_both_sides() {
    let log = DeviationLog {
        task: "B004".to_owned(),
        deviations: vec![deviation("extra")],
    };
    let problems = mismatch(&[deviation("missing")], Some(&log), "B003");
    assert_eq!(
        problems,
        vec![
            "docs/deviations/B003.yaml names task B004, not B003",
            "docs/deviations/B003.yaml is missing the report's deviation: missing",
            "docs/deviations/B003.yaml has a deviation the report doesn't list: extra",
        ]
    );
}
