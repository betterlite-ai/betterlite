use crate::credentials::contracts::CliTeam;
use crate::sso_polling::selection::prompt_team_id;

use super::fixtures::{Echo, FakeInput, piped_input, team};

fn teams() -> Vec<CliTeam> {
    vec![team("team-1", Some("Alpha")), team("team-2", Some("Beta"))]
}

#[test]
fn team_prompt_shows_the_table_and_returns_the_chosen_id() {
    let echo = Echo::default();
    let sink = echo.sink();
    let mut input = piped_input(vec!["2"]);

    let team_id = prompt_team_id(&teams(), &mut input, &sink).expect("the team id");

    assert_eq!(team_id, "team-2");
    let messages = echo.recorded();
    assert_eq!(messages[0], "Available Teams");
    assert_eq!(messages[1], "Index  Team Name  Team ID");
    assert_eq!(messages[2], "1  Alpha  team-1");
    assert_eq!(messages[3], "2  Beta  team-2");
    assert!(
        messages
            .iter()
            .any(|message| message == "\nSelected team: Beta (team-2)")
    );
}

#[rstest::rstest]
#[case::lowercase("skip")]
#[case::uppercase("SKIP")]
fn team_prompt_skip_uses_the_first_team(#[case] answer: &str) {
    let echo = Echo::default();
    let sink = echo.sink();
    let mut input = piped_input(vec![answer]);

    let team_id = prompt_team_id(&teams(), &mut input, &sink).expect("the team id");

    assert_eq!(team_id, "team-1");
}

#[test]
fn team_prompt_retries_an_out_of_range_index() {
    let echo = Echo::default();
    let sink = echo.sink();
    let mut input = piped_input(vec!["0", "1"]);

    let team_id = prompt_team_id(&teams(), &mut input, &sink).expect("the team id");

    assert_eq!(team_id, "team-1");
    assert!(
        echo.recorded()
            .iter()
            .any(|message| message.contains("between 1 and 2"))
    );
}

#[test]
fn team_prompt_retries_a_non_number() {
    let echo = Echo::default();
    let sink = echo.sink();
    let mut input = piped_input(vec!["nope", "1"]);

    let team_id = prompt_team_id(&teams(), &mut input, &sink).expect("the team id");

    assert_eq!(team_id, "team-1");
    assert!(
        echo.recorded()
            .iter()
            .any(|message| message == "Invalid input. Please enter a number or 'skip'")
    );
}

#[test]
fn team_prompt_falls_back_to_the_team_id_without_an_alias() {
    let echo = Echo::default();
    let sink = echo.sink();
    let mut input = piped_input(vec!["1"]);
    let unnamed = vec![team("team-2", None)];

    let team_id = prompt_team_id(&unnamed, &mut input, &sink).expect("the team id");

    assert_eq!(team_id, "team-2");
    let messages = echo.recorded();
    assert_eq!(messages[2], "1  team-2  team-2");
    assert!(
        messages
            .iter()
            .any(|message| message == "\nSelected team: team-2 (team-2)")
    );
}

#[test]
fn team_prompt_returns_none_without_teams() {
    let echo = Echo::default();
    let sink = echo.sink();
    let mut input = piped_input(vec!["1"]);

    assert_eq!(prompt_team_id(&[], &mut input, &sink), None);
    assert!(echo.recorded().is_empty());
}

#[test]
fn team_prompt_returns_none_at_end_of_input() {
    let echo = Echo::default();
    let sink = echo.sink();
    let mut input = FakeInput::default();

    assert_eq!(prompt_team_id(&teams(), &mut input, &sink), None);
}
