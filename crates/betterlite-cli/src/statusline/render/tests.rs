#![cfg(test)]

use std::fs;
use std::path::Path;

use rstest::rstest;
use rust_decimal::Decimal;
use tempfile::TempDir;

use super::{baseline_label, display_width, render};
use crate::statusline::session::Session;

const FULL: &str = "\u{2588}";
const EMPTY: &str = "\u{2591}";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[90m";
const RESET: &str = "\x1b[0m";
const ROUTER: &str = "\x1b[38;2;79;70;229m";
const BASELINE: &str = "\x1b[38;2;217;119;87m";
const NO_CACHE: &str = "/nonexistent/config";

fn dec(text: &str) -> Decimal {
    text.parse().unwrap()
}

fn base() -> Session {
    Session {
        router_name: "auto".to_owned(),
        last_model: "gpt-5".to_owned(),
        spend: dec("1"),
        baseline_spend: Some(dec("4")),
        baseline_model: Some("claude-opus-4".to_owned()),
        turns: None,
        savings_estimated_turns: None,
        savings_estimated_actual_spend: None,
    }
}

fn plain(session: &Session) -> String {
    render("gpt-5", Some(session), Path::new(NO_CACHE), false)
}

fn cells(filled: usize) -> String {
    format!(
        "{}{}",
        FULL.repeat(filled),
        EMPTY.repeat(24_usize.saturating_sub(filled))
    )
}

#[test]
fn no_session_prints_only_the_routed_line() {
    assert_eq!(
        render("gpt-5", None, Path::new(NO_CACHE), false),
        "Routed to: gpt-5"
    );
}

#[test]
fn the_routed_line_is_bold_in_color() {
    assert_eq!(
        render("gpt-5", None, Path::new(NO_CACHE), true),
        format!("{BOLD}Routed to: gpt-5{RESET}")
    );
}

#[rstest]
#[case::zero_estimated_turns(Session { savings_estimated_turns: Some(0), ..base() })]
#[case::no_baseline_spend(Session { baseline_spend: None, ..base() })]
#[case::zero_estimated_turns_and_no_baseline_spend(Session { savings_estimated_turns: Some(0), baseline_spend: None, ..base() })]
#[case::zero_estimated_turns_beats_a_missing_baseline_model(Session { savings_estimated_turns: Some(0), baseline_model: None, ..base() })]
#[case::zero_estimated_turns_beats_a_zero_baseline(Session { savings_estimated_turns: Some(0), baseline_spend: Some(dec("0")), ..base() })]
#[case::no_baseline_spend_beats_a_missing_baseline_model(Session { baseline_spend: None, baseline_model: None, ..base() })]
#[case::estimates_without_actual_spend(Session { savings_estimated_turns: Some(1), turns: Some(2), ..base() })]
#[case::estimates_without_turns(Session { savings_estimated_turns: Some(1), savings_estimated_actual_spend: Some(dec("1")), ..base() })]
#[case::estimates_over_turns(Session { savings_estimated_turns: Some(3), turns: Some(2), savings_estimated_actual_spend: Some(dec("1")), ..base() })]
fn savings_that_cannot_be_told_add_a_notice(#[case] session: Session) {
    assert_eq!(
        plain(&session),
        "Routed to: gpt-5 \u{b7} Savings unavailable"
    );
}

#[test]
fn the_notice_keeps_the_routed_line_bold_in_color() {
    let session = Session {
        baseline_spend: None,
        ..base()
    };

    assert_eq!(
        render("gpt-5", Some(&session), Path::new(NO_CACHE), true),
        format!("{BOLD}Routed to: gpt-5{RESET} \u{b7} Savings unavailable")
    );
}

#[rstest]
#[case::no_baseline_model(Session { baseline_model: None, ..base() })]
#[case::zero_baseline(Session { baseline_spend: Some(dec("0")), ..base() })]
#[case::negative_baseline(Session { baseline_spend: Some(dec("-1")), ..base() })]
#[case::no_baseline_model_with_valid_estimates(Session { baseline_model: None, savings_estimated_turns: Some(1), turns: Some(1), savings_estimated_actual_spend: Some(dec("1")), ..base() })]
fn a_baseline_that_cannot_be_drawn_prints_only_the_routed_line(#[case] session: Session) {
    assert_eq!(plain(&session), "Routed to: gpt-5");
}

#[test]
fn a_saving_draws_the_percentage_and_two_bars() {
    let expected = format!(
        "Routed to: gpt-5  -75% vs Claude Opus 4\nauto          {} $1.00\nClaude Opus 4 {} $4.00",
        cells(6),
        cells(24)
    );

    assert_eq!(plain(&base()), expected);
}

#[test]
fn color_paints_the_header_the_labels_the_bars_and_the_money() {
    let expected = format!(
        "{BOLD}Routed to: gpt-5{RESET}  {ROUTER}-75% vs Claude Opus 4{RESET}\n\
         {DIM}auto         {RESET} {ROUTER}{}{DIM}{}{RESET} {DIM}$1.00{RESET}\n\
         {DIM}Claude Opus 4{RESET} {BASELINE}{}{DIM}{}{RESET} {DIM}$4.00{RESET}",
        FULL.repeat(6),
        EMPTY.repeat(18),
        FULL.repeat(24),
        ""
    );

    assert_eq!(
        render("gpt-5", Some(&base()), Path::new(NO_CACHE), true),
        expected
    );
}

#[rstest]
#[case::saving("1", "4", "-75%")]
#[case::increase("6", "4", "+50%")]
#[case::equal("4", "4", "0%")]
#[case::free_session("0", "4", "-100%")]
#[case::spend_is_negative("-4", "16", "-125%")]
#[case::tie_rounds_down_to_even("7", "8", "-12%")]
#[case::tie_rounds_up_to_even("6.92", "8", "-14%")]
#[case::half_percent_is_zero("199", "200", "0%")]
#[case::one_and_a_half_percent_is_two("197", "200", "-2%")]
#[case::negative_tie("8.1", "8", "+1%")]
fn the_percentage_is_rounded_to_even_and_signed_against_the_baseline(
    #[case] spend: &str,
    #[case] baseline: &str,
    #[case] shown: &str,
) {
    let session = Session {
        spend: dec(spend),
        baseline_spend: Some(dec(baseline)),
        ..base()
    };

    let header = plain(&session).lines().next().unwrap().to_owned();

    assert_eq!(
        header,
        format!("Routed to: gpt-5  {shown} vs Claude Opus 4")
    );
}

#[rstest]
#[case::empty("0", 0)]
#[case::below_half_a_cell("0.5", 0)]
#[case::half_a_cell_rounds_to_even_zero("1", 0)]
#[case::one_and_a_half_rounds_to_even_two("3", 2)]
#[case::four_and_a_half_rounds_to_even_four("9", 4)]
#[case::seven_and_a_half_rounds_to_even_eight("15", 8)]
#[case::ten_and_a_half_rounds_to_even_ten("21", 10)]
#[case::full("48", 24)]
#[case::negative_spend_is_empty("-4", 0)]
fn a_bar_fills_to_the_nearest_even_cell_of_the_larger_amount(
    #[case] spend: &str,
    #[case] filled: usize,
) {
    let session = Session {
        spend: dec(spend),
        baseline_spend: Some(dec("48")),
        ..base()
    };

    let rendered = plain(&session);
    let router_row = rendered.lines().nth(1).unwrap();

    assert!(
        router_row.contains(&format!(" {} $", cells(filled))),
        "{router_row}"
    );
}

#[test]
fn the_larger_amount_sets_the_scale_when_spend_exceeds_the_baseline() {
    let session = Session {
        spend: dec("6"),
        ..base()
    };

    let rendered = plain(&session);

    let rows: Vec<&str> = rendered.lines().skip(1).collect();
    assert!(rows.first().unwrap().contains(&cells(24)));
    assert!(rows.last().unwrap().contains(&cells(16)));
}

#[rstest]
#[case::cents("1", "$1.00")]
#[case::pads_to_two_places("12.5", "$12.50")]
#[case::tie_to_even_down("0.125", "$0.12")]
#[case::tie_to_even_up("0.135", "$0.14")]
#[case::keeps_the_sign("-0.5", "$-0.50")]
fn money_is_shown_with_two_decimals(#[case] spend: &str, #[case] shown: &str) {
    let session = Session {
        spend: dec(spend),
        baseline_spend: Some(dec("40")),
        ..base()
    };

    let rendered = plain(&session);

    assert!(
        rendered.lines().nth(1).unwrap().ends_with(shown),
        "{rendered}"
    );
}

#[test]
fn estimates_compare_the_estimated_actual_spend_and_report_coverage() {
    let session = Session {
        spend: dec("10"),
        savings_estimated_turns: Some(1),
        turns: Some(2),
        savings_estimated_actual_spend: Some(dec("2")),
        ..base()
    };

    let rendered = plain(&session);

    let mut lines = rendered.lines();
    assert_eq!(
        lines.next().unwrap(),
        "Routed to: gpt-5  -50% vs Claude Opus 4 \u{b7} 1 of 2 turns estimated"
    );
    assert!(lines.next().unwrap().ends_with("$2.00"));
}

#[test]
fn estimates_covering_every_turn_still_draw_the_comparison() {
    let session = Session {
        savings_estimated_turns: Some(2),
        turns: Some(2),
        savings_estimated_actual_spend: Some(dec("1")),
        ..base()
    };

    assert!(
        plain(&session)
            .starts_with("Routed to: gpt-5  -75% vs Claude Opus 4 \u{b7} 2 of 2 turns estimated\n")
    );
}

#[test]
fn money_too_large_to_compare_reads_as_unavailable() {
    let session = Session {
        spend: dec("-1"),
        baseline_spend: Some(Decimal::MAX),
        ..base()
    };

    assert_eq!(
        plain(&session),
        "Routed to: gpt-5 \u{b7} Savings unavailable"
    );
}

#[test]
fn a_wide_router_name_is_padded_by_its_display_width() {
    let session = Session {
        router_name: "\u{30e2}\u{30c7}\u{30eb}".to_owned(),
        ..base()
    };

    let rendered = plain(&session);

    assert!(
        rendered
            .lines()
            .nth(1)
            .unwrap()
            .starts_with("\u{30e2}\u{30c7}\u{30eb}        ")
    );
    assert!(
        !rendered
            .lines()
            .nth(1)
            .unwrap()
            .starts_with("\u{30e2}\u{30c7}\u{30eb}         ")
    );
}

#[rstest]
#[case::ascii("abc", 3)]
#[case::empty("", 0)]
#[case::katakana("\u{30e2}\u{30c7}\u{30eb}", 6)]
#[case::hangul("\u{d55c}\u{ae00}", 4)]
#[case::fullwidth_latin("\u{ff21}", 2)]
#[case::emoji("\u{1f600}", 2)]
#[case::combining_mark("e\u{301}", 1)]
#[case::enclosing_mark("a\u{20dd}", 1)]
#[case::ambiguous_width_counts_one("\u{b1}", 1)]
#[case::mixed("a\u{30e2}\u{301}b", 4)]
fn display_width_counts_columns(#[case] label: &str, #[case] width: usize) {
    assert_eq!(display_width(label), width);
}

fn labelled(model: &str, entries: &str) -> String {
    let directory = TempDir::new().unwrap();
    let cache = directory.path().join("cache");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("gateway-models.json"), entries).unwrap();
    baseline_label(model, directory.path())
}

#[rstest]
#[case::capitalizes_words("claude-opus-4", r#"{"models":[]}"#, "Claude Opus 4")]
#[case::uses_the_bare_id("anthropic/claude-sonnet-4-5", r#"{"models":[]}"#, "Claude Sonnet 4 5")]
#[case::lowers_the_rest("GPT-5", r#"{"models":[]}"#, "Gpt 5")]
#[case::collapses_repeated_dashes("a--b", r#"{"models":[]}"#, "A B")]
#[case::display_name_is_kept(
    "claude-opus-4",
    r#"{"models":[{"id":"claude-opus-4","display_name":"Opus Four"}]}"#,
    "Opus Four"
)]
#[case::display_name_equal_to_the_id_is_prettified(
    "claude-opus-4",
    r#"{"models":[{"id":"claude-opus-4","display_name":"claude-opus-4"}]}"#,
    "Claude Opus 4"
)]
fn a_baseline_is_labelled_from_the_model_cache_or_prettified(
    #[case] model: &str,
    #[case] entries: &str,
    #[case] expected: &str,
) {
    assert_eq!(labelled(model, entries), expected);
}

#[test]
fn the_comparison_names_the_baseline_by_its_label() {
    let directory = TempDir::new().unwrap();
    let cache = directory.path().join("cache");
    fs::create_dir_all(&cache).unwrap();
    fs::write(
        cache.join("gateway-models.json"),
        r#"{"models":[{"id":"claude-opus-4","display_name":"Opus Four"}]}"#,
    )
    .unwrap();

    let rendered = render("gpt-5", Some(&base()), directory.path(), false);

    assert!(rendered.starts_with("Routed to: gpt-5  -75% vs Opus Four\n"));
    assert!(rendered.lines().last().unwrap().starts_with("Opus Four "));
}
