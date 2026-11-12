use std::io::Write;

use crate::commands::tables::write_table;

use super::ListedTeam;

const ALL_MODELS: &str = "All models";
const MEMBER: &str = "Member";
const NOT_AVAILABLE: &str = "N/A";

pub(super) fn display_teams(teams: &[ListedTeam], stdout: &mut dyn Write) {
    let rows = (1_usize..)
        .zip(teams)
        .map(|(index, team)| {
            vec![
                index.to_string(),
                team.team_alias
                    .clone()
                    .unwrap_or_else(|| String::from(NOT_AVAILABLE)),
                team.team_id
                    .clone()
                    .unwrap_or_else(|| String::from(NOT_AVAILABLE)),
                models_text(team.models.as_deref().unwrap_or_default()),
                team.max_budget
                    .map_or_else(|| String::from("Unlimited"), |amount| format!("${amount}")),
                String::from(MEMBER),
            ]
        })
        .collect();
    write_table(
        "Available Teams",
        &[
            "Index",
            "Team Alias",
            "Team ID",
            "Models",
            "Max Budget",
            "Role",
        ],
        rows,
        stdout,
    );
}

fn models_text(models: &[String]) -> String {
    if models.is_empty() {
        return String::from(ALL_MODELS);
    }
    match models.len().checked_sub(3) {
        Some(extra) if extra > 0 => {
            let first: Vec<&str> = models.iter().take(3).map(String::as_str).collect();
            format!("{} (+{extra} more)", first.join(", "))
        }
        _ => models.join(", "),
    }
}
