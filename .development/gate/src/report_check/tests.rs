#![cfg(test)]

use std::collections::BTreeSet;
use std::path::Path;

use crate::answer::parse_answer;
use crate::report_check::report_problems;
use crate::spec::report::Report;
use crate::spec::task::Task;
use crate::yaml::read_yaml;

const ENTRY: &str = r#"{"behavior":"each chunk keeps the id of the first chunk","litellm":"https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/streaming_handler.py#L100","code":"crates/betterlite-openai/src/stream.rs:40","test":"chunks_keep_the_first_id"}"#;

const DONE: &str = r#"{"task":"B005","status":"done","summary":"Streams chat completions as server-sent events.","blocker":"","owned":[{"behavior":"each chunk keeps the id of the first chunk","litellm":"https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/streaming_handler.py#L100","code":"crates/betterlite-openai/src/stream.rs:40","test":"chunks_keep_the_first_id"}],"prior_art":[{"need":"parse server-sent events split across reads","options":[{"name":"eventsource-stream","link":"https://docs.rs/eventsource-stream","verdict":"chosen","why":"buffers partial events"},{"name":"hand-written split","link":"https://html.spec.whatwg.org/multipage/server-sent-events.html","verdict":"rejected","why":"breaks on events split across reads"}]}],"deviations":[],"notes":[]}"#;

fn added(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn task(file: &str) -> Task {
    read_yaml(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tasks")
            .join(file),
    )
    .unwrap()
}

#[test]
fn a_new_dependency_chosen_in_prior_art_passes() {
    let report: Report = parse_answer(DONE, "test").unwrap();
    assert_eq!(
        report_problems(
            &report,
            &task("B005-chat-streaming.yaml"),
            &added(&["eventsource-stream"])
        ),
        Vec::<String>::new()
    );
}

#[test]
fn a_new_dependency_without_prior_art_fails() {
    let report: Report = parse_answer(DONE, "test").unwrap();
    let problems = report_problems(
        &report,
        &task("B005-chat-streaming.yaml"),
        &added(&["eventsource-stream", "bytes"]),
    );
    assert_eq!(
        problems,
        ["new dependency bytes has no prior_art entry that chose it over an alternative"]
    );
}

#[test]
fn a_single_option_is_not_a_comparison() {
    let single = DONE.replace(r#",{"name":"hand-written split","link":"https://html.spec.whatwg.org/multipage/server-sent-events.html","verdict":"rejected","why":"breaks on events split across reads"}"#, "");
    let report: Report = parse_answer(&single, "test").unwrap();
    assert_eq!(
        report_problems(&report, &task("B005-chat-streaming.yaml"), &added(&[])).len(),
        1
    );
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
        report_problems(
            &report,
            &task("B006-embeddings.yaml"),
            &added(&["eventsource-stream"])
        )
        .len(),
        1
    );
}

#[test]
fn a_done_report_lists_what_the_referenced_litellm_code_owns() {
    let empty = DONE.replace(ENTRY, "");
    let report: Report = parse_answer(&empty, "test").unwrap();
    assert_eq!(
        report_problems(
            &report,
            &task("B005-chat-streaming.yaml"),
            &added(&["eventsource-stream"])
        ),
        ["owned lists each behavior a user can observe in the LiteLLM code the references own"]
    );
}

#[test]
fn a_task_without_litellm_references_owns_nothing() {
    let empty = DONE
        .replace(ENTRY, "")
        .replace(r#""task":"B005""#, r#""task":"B1139""#);
    let report: Report = parse_answer(&empty, "test").unwrap();
    assert_eq!(
        report_problems(
            &report,
            &task("B1139-http-scenarios.yaml"),
            &added(&["eventsource-stream"])
        ),
        Vec::<String>::new()
    );
}

#[test]
fn an_owned_behavior_names_its_test() {
    let untested = DONE.replace(r#""test":"chunks_keep_the_first_id""#, r#""test":"""#);
    let report: Report = parse_answer(&untested, "test").unwrap();
    assert_eq!(
        report_problems(
            &report,
            &task("B005-chat-streaming.yaml"),
            &added(&["eventsource-stream"])
        ),
        [
            "owned 'each chunk keeps the id of the first chunk' needs an https link to the LiteLLM line, the code that does it and the test that proves it"
        ]
    );
}

#[test]
fn a_done_report_with_a_placeholder_summary_fails() {
    let placeholder = DONE.replace("Streams chat completions as server-sent events.", "test");
    let report: Report = parse_answer(&placeholder, "test").unwrap();
    assert_eq!(
        report_problems(
            &report,
            &task("B005-chat-streaming.yaml"),
            &added(&["eventsource-stream"])
        ),
        [
            "a done report needs a summary of at least 40 characters and an empty blocker; a blocked report needs a blocker"
        ]
    );
}
