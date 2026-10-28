#![cfg(test)]

use std::path::Path;

use crate::answer::parse_answer;
use crate::review_check::review_problems;
use crate::spec::review::Review;
use crate::spec::rules::Rules;
use crate::yaml::read_yaml;

const CHANGED: [&str; 1] = ["crates/betterlite-proxy/src/routes.rs"];

fn problems(text: &str) -> Vec<String> {
    let rules: Rules =
        read_yaml(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../rules.yaml")).unwrap();
    let review: Review = parse_answer(text, "test").unwrap();
    let changed = CHANGED.map(str::to_owned);
    review_problems(&review, "B005", &rules, &changed)
}

fn review(verdict: &str, origin: &str) -> String {
    format!(
        r#"{{"task":"B005","verdict":"{verdict}","findings":[{{"rule":"owned-behavior-is-built","origin":"{origin}","location":"crates/betterlite-proxy/src/routes.rs:12","problem":"A body that is not JSON gets a 500.","fix":"Answer 400."}}]}}"#
    )
}

#[test]
fn a_litellm_finding_does_not_hold_an_approval() {
    assert_eq!(
        problems(&review("approve", "litellm")),
        Vec::<String>::new()
    );
}

#[test]
fn changes_need_a_finding_of_origin_ours() {
    assert_eq!(
        problems(&review("changes", "litellm")),
        ["approve comes with no findings of origin ours and changes with at least one"]
    );
}

#[test]
fn an_approval_with_a_finding_of_origin_ours_is_malformed() {
    assert_eq!(problems(&review("approve", "ours")).len(), 1);
}

#[test]
fn a_finding_without_an_origin_is_rejected() {
    let text = review("changes", "ours").replace(r#""origin":"ours","#, "");
    assert!(
        parse_answer::<Review>(&text, "test")
            .is_err_and(|error| error.to_string().contains("origin"))
    );
}
