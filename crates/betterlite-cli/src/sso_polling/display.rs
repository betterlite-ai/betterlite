use anstyle::{AnsiColor, Color, Style};

use crate::credentials::contracts::CliTeam;
use crate::credentials::notice::MessageSink;

use super::CLEAR_SCREEN;

pub const SELECT_TEAM_HEADER: &str =
    "Select a Team (Use up/down arrows, Enter to select, 'q' to skip):";

const MAX_LISTED_MODELS: usize = 3;

const HIGHLIGHT: Style = Style::new()
    .bold()
    .fg_color(Some(Color::Ansi(AnsiColor::Cyan)));
const MODELS_STYLE: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Yellow)));
const BUDGET_STYLE: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Blue)));
const DIMMED: Style = Style::new().dimmed();

pub fn display_team_selection(
    teams: &[CliTeam],
    selected_index: usize,
    styled: bool,
    echo: MessageSink<'_>,
) {
    if styled {
        echo(CLEAR_SCREEN);
    }
    echo(SELECT_TEAM_HEADER);
    for (index, team) in teams.iter().enumerate() {
        let selected = index == selected_index;
        let marker = if selected { "> " } else { "  " };
        let alias = team.team_alias.as_deref().unwrap_or("N/A");
        let team_id = team.team_id.as_deref().unwrap_or("N/A");
        let alias_style = if selected { HIGHLIGHT } else { DIMMED };
        let models_style = if selected { MODELS_STYLE } else { DIMMED };
        let budget_style = if selected { BUDGET_STYLE } else { DIMMED };
        let budget = team
            .max_budget
            .map_or_else(|| String::from("Unlimited"), |budget| format!("${budget}"));
        echo(&format!(
            "{marker}{} ({team_id})",
            paint(styled, alias_style, alias)
        ));
        echo(&format!(
            "   Models: {}",
            paint(styled, models_style, &format_models(&team.models))
        ));
        echo(&format!(
            "   Budget: {}",
            paint(styled, budget_style, &budget)
        ));
        echo("");
    }
}

fn paint(styled: bool, style: Style, text: &str) -> String {
    if styled {
        format!("{style}{text}{style:#}")
    } else {
        text.to_owned()
    }
}

fn format_models(models: &[String]) -> String {
    if models.is_empty() {
        return String::from("All models");
    }
    if models.len() > MAX_LISTED_MODELS {
        let listed = models.get(..MAX_LISTED_MODELS).unwrap_or(models);
        let extra = models.len().saturating_sub(MAX_LISTED_MODELS);
        return format!("{} (+{extra} more)", listed.join(", "));
    }
    models.join(", ")
}
