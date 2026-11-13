use console::Key;

use crate::credentials::contracts::CliTeam;
use crate::sso_polling::selection::prompt_team_selection;

use super::fixtures::{Echo, interactive_input, piped_input, team, terminal_input};

fn teams() -> Vec<CliTeam> {
    vec![team("team-1", Some("Alpha")), team("team-2", Some("Beta"))]
}

#[test]
fn interactive_returns_none_without_teams() {
    let echo = Echo::default();
    let sink = echo.sink();
    let mut input = interactive_input(Vec::new());

    assert_eq!(prompt_team_selection(&[], &mut input, &sink), None);
    assert!(echo.recorded().is_empty());
}

#[test]
fn interactive_moves_down_and_returns_the_selected_team_on_enter() {
    let echo = Echo::default();
    let sink = echo.sink();
    let mut input = interactive_input(vec![Key::ArrowDown, Key::Enter]);

    let selected = prompt_team_selection(&teams(), &mut input, &sink).expect("the selected team");

    assert_eq!(selected.team_id.as_deref(), Some("team-2"));
    assert!(
        echo.recorded()
            .iter()
            .any(|message| message.contains("Selected team: Beta (team-2)"))
    );
}

#[test]
fn interactive_wraps_up_from_the_first_team_to_the_last() {
    let echo = Echo::default();
    let sink = echo.sink();
    let mut input = interactive_input(vec![Key::ArrowUp, Key::Enter]);

    let selected = prompt_team_selection(&teams(), &mut input, &sink).expect("the selected team");

    assert_eq!(selected.team_id.as_deref(), Some("team-2"));
}

#[test]
fn interactive_wraps_down_past_the_last_team() {
    let echo = Echo::default();
    let sink = echo.sink();
    let mut input = interactive_input(vec![Key::ArrowDown, Key::ArrowDown, Key::Enter]);

    let selected = prompt_team_selection(&teams(), &mut input, &sink).expect("the selected team");

    assert_eq!(selected.team_id.as_deref(), Some("team-1"));
}

#[test]
fn interactive_escape_skips_the_selection() {
    let echo = Echo::default();
    let sink = echo.sink();
    let mut input = interactive_input(vec![Key::Escape]);

    assert_eq!(prompt_team_selection(&teams(), &mut input, &sink), None);
    assert!(
        echo.recorded()
            .iter()
            .any(|message| message == "Team selection skipped.")
    );
}

#[test]
fn interactive_quit_skips_the_selection() {
    let echo = Echo::default();
    let sink = echo.sink();
    let mut input = interactive_input(vec![Key::Char('q')]);

    assert_eq!(prompt_team_selection(&teams(), &mut input, &sink), None);
}

#[test]
fn interactive_without_a_terminal_uses_the_number_prompt() {
    let echo = Echo::default();
    let sink = echo.sink();
    let mut input = piped_input(vec!["2"]);

    let selected = prompt_team_selection(&teams(), &mut input, &sink).expect("the selected team");

    assert_eq!(selected.team_id.as_deref(), Some("team-2"));
}

#[test]
fn interactive_unknown_key_uses_the_number_prompt() {
    let echo = Echo::default();
    let sink = echo.sink();
    let mut input = terminal_input(vec![Key::Unknown], vec!["1"]);

    let selected = prompt_team_selection(&teams(), &mut input, &sink).expect("the selected team");

    assert_eq!(selected.team_id.as_deref(), Some("team-1"));
}
