use rstest::rstest;

use crate::credentials::contracts::CliTeam;
use crate::sso_polling::selection::prompt_team_selection_fallback;

use super::fixtures::{Echo, FakeInput, piped_input, team};

fn teams() -> Vec<CliTeam> {
    vec![team("team-1", Some("Alpha")), team("team-2", Some("Beta"))]
}

#[test]
fn fallback_selects_the_team_at_the_entered_one_indexed_number() {
    let echo = Echo::default();
    let sink = echo.sink();
    let mut input = piped_input(vec!["2"]);

    let selected =
        prompt_team_selection_fallback(&teams(), &mut input, &sink).expect("the selected team");

    assert_eq!(selected.team_id.as_deref(), Some("team-2"));
    assert!(
        echo.recorded()
            .iter()
            .any(|message| message.contains("(or 'skip' to continue without a team)"))
    );
}

#[rstest]
#[case::lowercase("skip")]
#[case::uppercase("SKIP")]
#[case::mixed("Skip")]
fn fallback_returns_none_for_skip(#[case] answer: &str) {
    let echo = Echo::default();
    let sink = echo.sink();
    let mut input = piped_input(vec![answer]);

    assert_eq!(
        prompt_team_selection_fallback(&teams(), &mut input, &sink),
        None
    );
}

#[test]
fn fallback_reprompts_after_a_non_number() {
    let echo = Echo::default();
    let sink = echo.sink();
    let mut input = piped_input(vec!["abc", "1"]);

    let selected =
        prompt_team_selection_fallback(&teams(), &mut input, &sink).expect("the selected team");

    assert_eq!(selected.team_id.as_deref(), Some("team-1"));
    assert!(
        echo.recorded()
            .iter()
            .any(|message| message == "Invalid input. Please enter a number or 'skip'")
    );
}

#[test]
fn fallback_reprompts_after_an_out_of_range_number() {
    let echo = Echo::default();
    let sink = echo.sink();
    let mut input = piped_input(vec!["7", "1"]);

    let selected =
        prompt_team_selection_fallback(&teams(), &mut input, &sink).expect("the selected team");

    assert_eq!(selected.team_id.as_deref(), Some("team-1"));
    assert!(
        echo.recorded()
            .iter()
            .any(|message| message.contains("between 1 and 2"))
    );
}

#[test]
fn fallback_reprompts_when_the_index_is_one_past_the_end() {
    let echo = Echo::default();
    let sink = echo.sink();
    let mut input = piped_input(vec!["3", "1"]);

    let selected =
        prompt_team_selection_fallback(&teams(), &mut input, &sink).expect("the selected team");

    assert_eq!(selected.team_id.as_deref(), Some("team-1"));
    assert!(
        echo.recorded()
            .iter()
            .any(|message| message.contains("between 1 and 2"))
    );
}

#[test]
fn fallback_treats_a_negative_number_as_out_of_range() {
    let echo = Echo::default();
    let sink = echo.sink();
    let mut input = piped_input(vec!["-1", "1"]);

    let selected =
        prompt_team_selection_fallback(&teams(), &mut input, &sink).expect("the selected team");

    assert_eq!(selected.team_id.as_deref(), Some("team-1"));
    assert!(
        echo.recorded()
            .iter()
            .any(|message| message.contains("between 1 and 2"))
    );
}

#[test]
fn fallback_returns_none_at_end_of_input() {
    let echo = Echo::default();
    let sink = echo.sink();
    let mut input = FakeInput::default();

    assert_eq!(
        prompt_team_selection_fallback(&teams(), &mut input, &sink),
        None
    );
}

#[test]
fn fallback_returns_none_without_teams() {
    let echo = Echo::default();
    let sink = echo.sink();
    let mut input = piped_input(vec!["1"]);

    assert_eq!(prompt_team_selection_fallback(&[], &mut input, &sink), None);
    assert!(echo.recorded().is_empty());
}
