use crate::credentials::contracts::CliTeam;
use crate::credentials::notice::MessageSink;

use super::CLEAR_SCREEN;
use super::display::display_team_selection;
use super::terminal::{KeyPress, SelectionInput, get_key_input};

const FALLBACK_PROMPT: &str =
    "\nSelect a team by entering the index number (or 'skip' to continue without a team)";
const LOGIN_TEAM_PROMPT: &str =
    "\nSelect a team by entering the index number (or 'skip' to use first team)";
const INVALID_INPUT_MESSAGE: &str = "Invalid input. Please enter a number or 'skip'";
const SKIPPED_MESSAGE: &str = "Team selection skipped.";
const TABLE_HEADER: &str = "Available Teams";
const TABLE_COLUMNS: &str = "Index  Team Name  Team ID";

#[must_use]
pub fn prompt_team_selection(
    teams: &[CliTeam],
    input: &mut dyn SelectionInput,
    echo: MessageSink<'_>,
) -> Option<CliTeam> {
    if teams.is_empty() {
        return None;
    }
    if !input.is_interactive() {
        return prompt_team_selection_fallback(teams, input, echo);
    }
    let mut selected_index = 0_usize;
    loop {
        display_team_selection(teams, selected_index, true, echo);
        match get_key_input(input) {
            Some(KeyPress::Up) => {
                selected_index = if selected_index == 0 {
                    teams.len().saturating_sub(1)
                } else {
                    selected_index.saturating_sub(1)
                };
            }
            Some(KeyPress::Down) => {
                selected_index = match selected_index.checked_add(1) {
                    Some(next) if next < teams.len() => next,
                    _ => 0,
                };
            }
            Some(KeyPress::Enter) => {
                let Some(team) = teams.get(selected_index) else {
                    return prompt_team_selection_fallback(teams, input, echo);
                };
                echo(CLEAR_SCREEN);
                echo(&selected_team_message(team));
                return Some(team.clone());
            }
            Some(KeyPress::Escape | KeyPress::Quit) => {
                echo(CLEAR_SCREEN);
                echo(SKIPPED_MESSAGE);
                return None;
            }
            None => return prompt_team_selection_fallback(teams, input, echo),
        }
    }
}

#[must_use]
pub fn prompt_team_selection_fallback(
    teams: &[CliTeam],
    input: &mut dyn SelectionInput,
    echo: MessageSink<'_>,
) -> Option<CliTeam> {
    if teams.is_empty() {
        return None;
    }
    loop {
        let answer = ask(input, echo, FALLBACK_PROMPT)?;
        let choice = answer.trim();
        if choice.eq_ignore_ascii_case("skip") {
            return None;
        }
        let Ok(number) = choice.parse::<i64>() else {
            echo(INVALID_INPUT_MESSAGE);
            continue;
        };
        let selected = usize::try_from(number)
            .ok()
            .and_then(|index| index.checked_sub(1))
            .and_then(|offset| teams.get(offset));
        let Some(team) = selected else {
            echo(&format!(
                "Invalid selection. Please enter a number between 1 and {}",
                teams.len()
            ));
            continue;
        };
        echo(&selected_team_message(team));
        return Some(team.clone());
    }
}

#[must_use]
pub fn prompt_team_id(
    teams: &[CliTeam],
    input: &mut dyn SelectionInput,
    echo: MessageSink<'_>,
) -> Option<String> {
    if teams.is_empty() {
        return None;
    }
    display_team_table(teams, echo);
    loop {
        let answer = ask(input, echo, LOGIN_TEAM_PROMPT)?;
        let choice = answer.trim();
        if choice.eq_ignore_ascii_case("skip") {
            let first_team = teams.first()?;
            return first_team.team_id.clone();
        }
        let Ok(number) = choice.parse::<i64>() else {
            echo(INVALID_INPUT_MESSAGE);
            continue;
        };
        let selected = usize::try_from(number)
            .ok()
            .and_then(|index| index.checked_sub(1))
            .and_then(|offset| teams.get(offset));
        let Some(team) = selected else {
            echo(&format!(
                "Invalid selection. Please enter a number between 1 and {}",
                teams.len()
            ));
            continue;
        };
        echo(&selected_login_team_message(team));
        return team.team_id.clone();
    }
}

fn display_team_table(teams: &[CliTeam], echo: MessageSink<'_>) {
    echo(TABLE_HEADER);
    echo(TABLE_COLUMNS);
    for (position, team) in (1_usize..).zip(teams) {
        let team_id = team.team_id.as_deref().unwrap_or("N/A");
        let name = team.team_alias.as_deref().unwrap_or(team_id);
        echo(&format!("{position}  {name}  {team_id}"));
    }
}

fn ask(input: &mut dyn SelectionInput, echo: MessageSink<'_>, prompt: &str) -> Option<String> {
    echo(prompt);
    input.read_line()
}

fn selected_team_message(team: &CliTeam) -> String {
    let alias = team.team_alias.as_deref().unwrap_or("N/A");
    let team_id = team.team_id.as_deref().unwrap_or("N/A");
    format!("\nSelected team: {alias} ({team_id})")
}

fn selected_login_team_message(team: &CliTeam) -> String {
    let team_id = team.team_id.as_deref().unwrap_or("N/A");
    let alias = team.team_alias.as_deref().unwrap_or(team_id);
    format!("\nSelected team: {alias} ({team_id})")
}
