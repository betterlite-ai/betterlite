#![cfg(test)]

use crate::deviations::mismatch;
use crate::spec::deviations::DeviationLog;
use crate::spec::report::ReportDeviation;

fn deviation(what: &str) -> ReportDeviation {
    ReportDeviation {
        what: what.to_owned(),
        why: "LiteLLM drops the header".to_owned(),
        link: format!("https://example.com/{what}"),
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
fn a_log_in_other_words_passes() {
    let mut logged = deviation("same");
    logged.what = "the same deviation in other words".to_owned();
    logged.why = "LiteLLM keeps the header".to_owned();
    let log = DeviationLog {
        task: "B003".to_owned(),
        deviations: vec![logged],
    };
    assert!(mismatch(&[deviation("same")], Some(&log), "B003").is_empty());
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
            "the report and docs/deviations/B003.yaml list different deviations, by count or link; report: missing (https://example.com/missing); docs/deviations/B003.yaml: extra (https://example.com/extra)",
        ]
    );
}

#[test]
fn a_log_with_one_more_entry_fails() {
    let log = DeviationLog {
        task: "B003".to_owned(),
        deviations: vec![deviation("a"), deviation("b")],
    };
    assert_eq!(mismatch(&[deviation("a")], Some(&log), "B003").len(), 1);
}
