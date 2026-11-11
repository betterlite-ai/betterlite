#![cfg(test)]

use std::collections::BTreeSet;

use crate::report_check::mutants::untested;
use crate::spec::report::MutantTest;

const LINE: &str = "crates/betterlite-paths/src/lib.rs:12:5: replace > with >= in limit";

fn entry(mutant: &str, test: &str) -> MutantTest {
    MutantTest {
        mutant: mutant.to_owned(),
        test: test.to_owned(),
    }
}

fn functions() -> BTreeSet<&'static str> {
    BTreeSet::from(["limit", "a_limit_of_ten_keeps_ten"])
}

#[test]
fn a_mutant_with_a_known_test_passes() {
    let entries = [entry(
        LINE,
        "betterlite-paths::tests::a_limit_of_ten_keeps_ten",
    )];
    assert_eq!(
        untested(&[LINE], &entries, &functions()),
        Vec::<String>::new()
    );
}

#[test]
fn a_mutant_the_report_leaves_out_fails() {
    let problems = untested(&[LINE], &[], &functions());
    assert_eq!(
        problems.first().map(String::as_str),
        Some(format!("mutants has no entry for `{LINE}`").as_str())
    );
    assert_eq!(problems.len(), 2);
}

#[test]
fn a_test_name_no_fn_declares_fails() {
    let entries = [entry(LINE, "tests::made_up_name")];
    let problems = untested(&[LINE], &entries, &functions());
    assert_eq!(
        problems.first().map(String::as_str),
        Some(
            format!("mutants names test `tests::made_up_name` for `{LINE}`, and no fn in the Rust sources has a name from that path")
                .as_str()
        )
    );
}

#[test]
fn an_entry_matches_after_trimming() {
    let entries = [entry(&format!("  {LINE} "), "a_limit_of_ten_keeps_ten")];
    assert_eq!(
        untested(&[LINE], &entries, &functions()),
        Vec::<String>::new()
    );
}

#[test]
fn only_twenty_mutants_are_shown() {
    let lines: Vec<String> = (0_i32..25_i32)
        .map(|line| format!("crates/betterlite-paths/src/lib.rs:{line}:5: replace foo with ()"))
        .collect();
    let listed: Vec<&str> = lines.iter().map(String::as_str).collect();
    let problems = untested(&listed, &[], &functions());
    assert_eq!(problems.len(), 22);
    assert_eq!(
        problems.get(20).map(String::as_str),
        Some("and 5 more mutants like these")
    );
}
