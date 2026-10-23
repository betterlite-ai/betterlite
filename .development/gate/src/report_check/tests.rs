#![cfg(test)]

use std::collections::BTreeSet;

use crate::answer::parse_answer;
use crate::report_check::report_problems;
use crate::spec::report::Report;

const DONE: &str = r#"{"task":"B005","status":"done","summary":"Streams chat completions.","blocker":"","prior_art":[{"need":"parse server-sent events split across reads","options":[{"name":"eventsource-stream","link":"https://docs.rs/eventsource-stream","verdict":"chosen","why":"buffers partial events"},{"name":"hand-written split","link":"https://html.spec.whatwg.org/multipage/server-sent-events.html","verdict":"rejected","why":"breaks on events split across reads"}]}],"deviations":[],"notes":[]}"#;

fn added(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn a_new_dependency_chosen_in_prior_art_passes() {
    let report: Report = parse_answer(DONE, "test").unwrap();
    assert_eq!(
        report_problems(&report, "B005", &added(&["eventsource-stream"])),
        Vec::<String>::new()
    );
}

#[test]
fn a_new_dependency_without_prior_art_fails() {
    let report: Report = parse_answer(DONE, "test").unwrap();
    let problems = report_problems(&report, "B005", &added(&["eventsource-stream", "bytes"]));
    assert_eq!(
        problems,
        ["new dependency bytes has no prior_art entry that chose it over an alternative"]
    );
}

#[test]
fn a_single_option_is_not_a_comparison() {
    let single = DONE.replace(r#",{"name":"hand-written split","link":"https://html.spec.whatwg.org/multipage/server-sent-events.html","verdict":"rejected","why":"breaks on events split across reads"}"#, "");
    let report: Report = parse_answer(&single, "test").unwrap();
    assert_eq!(report_problems(&report, "B005", &added(&[])).len(), 1);
}

#[test]
fn a_fenced_answer_still_parses() {
    let fenced = format!("```json\n{DONE}\n```");
    assert!(parse_answer::<Report>(&fenced, "test").is_ok_and(|report| report.task == "B005"));
}

#[test]
fn the_report_must_name_the_branch_task() {
    let report: Report = parse_answer(DONE, "test").unwrap();
    assert_eq!(
        report_problems(&report, "B006", &added(&["eventsource-stream"])).len(),
        1
    );
}
