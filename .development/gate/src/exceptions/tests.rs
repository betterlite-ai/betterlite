#![cfg(test)]

use crate::exceptions::{
    Blamed, BudgetNote, blamed_lines, budget_notes, csv_line, merge_dates, pull_number,
};

#[test]
fn budget_notes_take_the_paragraph_under_each_budget_line() {
    let log = "\u{1e}aaa\nB3814: Enforce stored tool policies (#1559)\n\n* feat(guardrails): enforce policies\n\n* docs(tasks): B3814 budget 1 expect\n\nThe helper takes a bare dict,\nso it spends one line.\n\n* refactor(guardrails): drop a check\n\u{1e}bbb\ndocs(tasks): B900 budget 700 to 900 lines\n\nThe parser needs two more files.\n";
    assert_eq!(
        budget_notes(log),
        [
            BudgetNote {
                task: "B3814".to_owned(),
                commit: "aaa".to_owned(),
                reason: "The helper takes a bare dict, so it spends one line.".to_owned(),
            },
            BudgetNote {
                task: "B900".to_owned(),
                commit: "bbb".to_owned(),
                reason: "The parser needs two more files.".to_owned(),
            },
        ]
    );
}

#[test]
fn a_budget_line_without_a_paragraph_has_no_reason() {
    let log = "\u{1e}ccc\nB12: Title (#3)\n\n* docs(tasks): B12 budget two expects\n* docs(backlog): B12 done\n";
    assert_eq!(
        budget_notes(log),
        [BudgetNote {
            task: "B12".to_owned(),
            commit: "ccc".to_owned(),
            reason: String::new(),
        }]
    );
}

#[test]
fn merge_dates_keep_the_newest_merge_of_each_pull_request() {
    let log = "2026-11-13 B724: Team rows (#1550)\n2026-11-12 dev: no number\n2026-11-10 B724: Team rows (#1550)\n2026-11-09 B1: First (#7)\n";
    let dates = merge_dates(log);
    assert_eq!(dates.get(&1550).map(String::as_str), Some("2026-11-13"));
    assert_eq!(dates.get(&7).map(String::as_str), Some("2026-11-09"));
    assert_eq!(dates.len(), 2);
    assert_eq!(pull_number("dev: shards (#1578) and more"), None);
}

#[test]
fn blamed_lines_carry_the_commit_and_summary_of_each_line() {
    let first = "a".repeat(40);
    let second = "b".repeat(40);
    let porcelain = format!(
        "{first} 1 1 1\nauthor Max\nsummary dev: deny duplicates (#900)\nfilename deny.toml\n\t\"aead@0.5.2\",\n{second} 2 2 1\nsummary B3078: add tokenizers (#950)\nfilename deny.toml\n\t\"nom@7.1.3\",\n"
    );
    assert_eq!(
        blamed_lines(&porcelain),
        [
            Blamed {
                line: "\"aead@0.5.2\",".to_owned(),
                commit: first,
                summary: "dev: deny duplicates (#900)".to_owned(),
            },
            Blamed {
                line: "\"nom@7.1.3\",".to_owned(),
                commit: second,
                summary: "B3078: add tokenizers (#950)".to_owned(),
            },
        ]
    );
}

#[test]
fn csv_cells_are_quoted_with_doubled_quotes() {
    assert_eq!(
        csv_line(&["expects", "B1", "say \"hi\", then go"]),
        "\"expects\",\"B1\",\"say \"\"hi\"\", then go\""
    );
}
