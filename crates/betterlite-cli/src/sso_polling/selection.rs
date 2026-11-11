use std::iter::from_fn;

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

enum Picked {
    Team(CliTeam),
    Skipped,
    Ended,
}

#[derive(Clone, Copy)]
enum PromptKind {
    Fallback,
    Login,
}

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
                echo(&selected_team_message(team, PromptKind::Fallback));
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

impl Picked {
    fn announced(
        self,
        kind: PromptKind,
        teams: &[CliTeam],
        echo: MessageSink<'_>,
    ) -> Option<CliTeam> {
        match self {
            Self::Team(team) => {
                echo(&selected_team_message(&team, kind));
                Some(team)
            }
            Self::Skipped => match kind {
                PromptKind::Fallback => None,
                PromptKind::Login => teams.first().cloned(),
            },
            Self::Ended => None,
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
    ask_for_choice(teams, input, echo, FALLBACK_PROMPT).announced(PromptKind::Fallback, teams, echo)
}

#[must_use]
pub fn prompt_team_id(
    choices: &[CliTeam],
    terminal: &mut dyn SelectionInput,
    sink: MessageSink<'_>,
) -> Option<String> {
    display_team_table(choices, sink);
    let picked = ask_for_choice(choices, terminal, sink, LOGIN_TEAM_PROMPT);
    picked.announced(PromptKind::Login, choices, sink)?.team_id
}

fn ask_for_choice(
    teams: &[CliTeam],
    input: &mut dyn SelectionInput,
    echo: MessageSink<'_>,
    prompt: &str,
) -> Picked {
    if teams.is_empty() {
        return Picked::Ended;
    }
    from_fn(|| {
        echo(prompt);
        input.read_line()
    })
    .find_map(|answer| interpret(teams, answer.trim(), echo))
    .unwrap_or(Picked::Ended)
}

fn interpret(teams: &[CliTeam], answer: &str, echo: MessageSink<'_>) -> Option<Picked> {
    if answer.eq_ignore_ascii_case("skip") {
        return Some(Picked::Skipped);
    }
    let Ok(number) = answer.parse::<i64>() else {
        echo(INVALID_INPUT_MESSAGE);
        return None;
    };
    let found = usize::try_from(number)
        .ok()
        .and_then(|index| index.checked_sub(1))
        .filter(|offset| *offset < teams.len());
    let Some(offset) = found else {
        echo(&format!(
            "Invalid selection. Please enter a number between 1 and {}",
            teams.len()
        ));
        return None;
    };
    teams.get(offset).cloned().map(Picked::Team)
}

fn display_team_table(teams: &[CliTeam], echo: MessageSink<'_>) {
    if teams.is_empty() {
        return;
    }
    echo(TABLE_HEADER);
    echo(TABLE_COLUMNS);
    for (position, team) in (1_usize..).zip(teams) {
        let team_id = team.team_id.as_deref().unwrap_or("N/A");
        let name = team.team_alias.as_deref().unwrap_or(team_id);
        echo(&format!("{position}  {name}  {team_id}"));
    }
}

fn selected_team_message(team: &CliTeam, kind: PromptKind) -> String {
    let team_id = team.team_id.as_deref().unwrap_or("N/A");
    let alias = match kind {
        PromptKind::Fallback => team.team_alias.as_deref().unwrap_or("N/A"),
        PromptKind::Login => team.team_alias.as_deref().unwrap_or(team_id),
    };
    format!("\nSelected team: {alias} ({team_id})")
}
