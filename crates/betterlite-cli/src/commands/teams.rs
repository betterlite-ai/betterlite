use std::fmt;
use std::io::Write;

use clap::{Args, Subcommand};
use http::Method;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

use crate::credentials::contracts::CliTeam;
use crate::credentials::notice::MessageSink;
use crate::sso_polling::selection::prompt_team_selection;
use crate::sso_polling::terminal::SelectionInput;

use super::{Client, Payload, Request, print_http_error, write_table};

const LIST_PATH: &str = "/team/list";
const AVAILABLE_PATH: &str = "/team/available";
const UPDATE_PATH: &str = "/key/update";
const NO_TEAMS: &str = "No teams found for your user.";
const NO_AVAILABLE_TEAMS: &str = "No available teams to join.";
const NO_API_KEY: &str = "No API key found. Please login first using 'lite login'";
const OPERATION_CANCELLED: &str = "Operation cancelled.";
const ALL_MODELS: &str = "All models";
const MEMBER: &str = "Member";
const NOT_AVAILABLE: &str = "N/A";

#[derive(Debug, Subcommand)]
#[command(about = "Manage teams and team assignments")]
pub enum TeamsCommand {
    List(ListTeams),
    Available(AvailableTeams),
    AssignKey(AssignKey),
}

#[derive(Debug, Args)]
pub struct ListTeams;

#[derive(Debug, Args)]
pub struct AvailableTeams;

#[derive(Debug, Args)]
pub struct AssignKey {
    #[arg(long = "team-id", help = "Team ID to assign the key to")]
    team_id: Option<String>,
}

pub struct TeamsPorts<'port> {
    pub client: &'port Client,
    pub terminal: &'port mut dyn SelectionInput,
    pub echo: MessageSink<'port>,
    pub stdout: &'port mut dyn Write,
    pub stderr: &'port mut dyn Write,
}

#[derive(Debug, thiserror::Error)]
pub enum TeamsError {
    #[error("the command was aborted")]
    Aborted,
}

#[derive(Debug, Deserialize)]
struct ListedTeam {
    #[serde(default)]
    team_id: Option<String>,
    #[serde(default)]
    team_alias: Option<String>,
    #[serde(default)]
    models: Option<Vec<String>>,
    #[serde(default)]
    max_budget: Option<Decimal>,
}

#[derive(Serialize)]
struct KeyUpdateBody<'request> {
    key: &'request str,
    team_id: &'request str,
}

pub fn run_teams_command(command: TeamsCommand, ports: TeamsPorts<'_>) -> Result<(), TeamsError> {
    match command {
        TeamsCommand::List(options) => list_teams(&options, ports),
        TeamsCommand::Available(options) => available_teams(&options, ports),
        TeamsCommand::AssignKey(options) => assign_key(&options, ports),
    }
}

fn list_teams(_options: &ListTeams, ports: TeamsPorts<'_>) -> Result<(), TeamsError> {
    let TeamsPorts {
        client,
        stdout,
        stderr,
        ..
    } = ports;
    let Ok(url) = client.resolve(LIST_PATH) else {
        return Err(report_failure(stderr, &LIST_PATH));
    };
    let request = Request {
        method: Method::GET,
        url,
        headers: vec![(
            String::from("content-type"),
            String::from("application/json"),
        )],
        payload: Payload::Empty,
    };
    let response = match client.send(request) {
        Ok(response) => response,
        Err(source) => return Err(report_failure(stderr, &source)),
    };
    if response.status >= 400 {
        print_http_error(response.status, &response.body, stdout, stderr);
        return Err(TeamsError::Aborted);
    }
    let teams = match serde_json::from_slice::<Vec<ListedTeam>>(&response.body) {
        Ok(teams) => teams,
        Err(source) => return Err(report_failure(stderr, &source)),
    };
    if teams.is_empty() {
        let _wrote_no_teams = writeln!(stdout, "{NO_TEAMS}");
        return Ok(());
    }
    display_teams(&teams, stdout);
    Ok(())
}

fn available_teams(_options: &AvailableTeams, ports: TeamsPorts<'_>) -> Result<(), TeamsError> {
    let TeamsPorts {
        client,
        stdout,
        stderr,
        ..
    } = ports;
    let Ok(url) = client.resolve(AVAILABLE_PATH) else {
        return Err(report_failure(stderr, &AVAILABLE_PATH));
    };
    let request = Request {
        method: Method::GET,
        url,
        headers: vec![(
            String::from("content-type"),
            String::from("application/json"),
        )],
        payload: Payload::Empty,
    };
    let response = match client.send(request) {
        Ok(response) => response,
        Err(source) => return Err(report_failure(stderr, &source)),
    };
    if response.status >= 400 {
        print_http_error(response.status, &response.body, stdout, stderr);
        return Ok(());
    }
    let teams = match serde_json::from_slice::<Vec<ListedTeam>>(&response.body) {
        Ok(teams) => teams,
        Err(source) => return Err(report_failure(stderr, &source)),
    };
    if teams.is_empty() {
        let _wrote_no_available = writeln!(stdout, "{NO_AVAILABLE_TEAMS}");
        return Ok(());
    }
    let _wrote_header = writeln!(stdout, "\nAvailable Teams to Join:");
    display_teams(&teams, stdout);
    Ok(())
}

fn assign_key(options: &AssignKey, ports: TeamsPorts<'_>) -> Result<(), TeamsError> {
    let TeamsPorts {
        client,
        terminal,
        echo,
        stdout,
        stderr,
    } = ports;
    let Some(api_key) = client.api_key.as_ref() else {
        let _wrote_no_key = writeln!(stdout, "{NO_API_KEY}");
        return Err(TeamsError::Aborted);
    };
    let Some(team_id) = team_id_to_assign(
        options,
        TeamsPorts {
            client,
            terminal,
            echo,
            stdout: &mut *stdout,
            stderr: &mut *stderr,
        },
    )?
    else {
        return Ok(());
    };
    update_key(client, api_key, &team_id, &mut *stdout, &mut *stderr)?;
    let teams = fetch_teams(client, &mut *stdout, &mut *stderr)?;
    announce_models(&teams, &team_id, stdout);
    Ok(())
}

fn team_id_to_assign(
    options: &AssignKey,
    ports: TeamsPorts<'_>,
) -> Result<Option<String>, TeamsError> {
    if let Some(given) = &options.team_id {
        return Ok(Some(given.clone()));
    }
    let TeamsPorts {
        client,
        terminal,
        echo,
        stdout,
        stderr,
    } = ports;
    let teams = fetch_teams(client, stdout, stderr)?;
    if teams.is_empty() {
        let _wrote_no_teams = writeln!(stdout, "{NO_TEAMS}");
        return Ok(None);
    }
    let prompt_teams: Vec<CliTeam> = teams
        .iter()
        .map(|team| CliTeam {
            team_id: team.team_id.clone(),
            team_alias: team.team_alias.clone(),
            models: team.models.clone().unwrap_or_default(),
            max_budget: team.max_budget,
        })
        .collect();
    let Some(selected) = prompt_team_selection(&prompt_teams, terminal, echo) else {
        let _wrote_cancelled = writeln!(stdout, "{OPERATION_CANCELLED}");
        return Ok(None);
    };
    Ok(selected.team_id)
}

fn update_key(
    client: &Client,
    api_key: &SecretString,
    team_id: &str,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), TeamsError> {
    let _wrote_assigning = writeln!(stdout, "\nAssigning your key to team: {team_id}");
    let body = serde_json::to_vec(&KeyUpdateBody {
        key: api_key.expose_secret(),
        team_id,
    });
    let encoded_body = match body {
        Ok(encoded) => encoded,
        Err(source) => return Err(report_failure(stderr, &source)),
    };
    let Ok(url) = client.resolve(UPDATE_PATH) else {
        return Err(report_failure(stderr, &UPDATE_PATH));
    };
    let request = Request {
        method: Method::POST,
        url,
        headers: Vec::new(),
        payload: Payload::Json(encoded_body),
    };
    let response = match client.send(request) {
        Ok(response) => response,
        Err(source) => return Err(report_failure(stderr, &source)),
    };
    if response.status >= 400 {
        print_http_error(response.status, &response.body, stdout, stderr);
        return Err(TeamsError::Aborted);
    }
    let _wrote_assigned = writeln!(stdout, "Successfully assigned key to team: {team_id}");
    Ok(())
}

fn announce_models(teams: &[ListedTeam], team_id: &str, stdout: &mut dyn Write) {
    for team in teams {
        if team.team_id.as_deref() == Some(team_id) {
            let models = team.models.as_deref().unwrap_or_default().join(", ");
            if models.is_empty() {
                let _wrote_all_models = writeln!(stdout, "You can now access all available models");
            } else {
                let _wrote_models = writeln!(stdout, "You can now access models: {models}");
            }
            break;
        }
    }
}

fn fetch_teams(
    client: &Client,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<Vec<ListedTeam>, TeamsError> {
    let Ok(url) = client.resolve(LIST_PATH) else {
        return Err(report_failure(stderr, &LIST_PATH));
    };
    let request = Request {
        method: Method::GET,
        url,
        headers: vec![(
            String::from("content-type"),
            String::from("application/json"),
        )],
        payload: Payload::Empty,
    };
    let response = match client.send(request) {
        Ok(response) => response,
        Err(source) => return Err(report_failure(stderr, &source)),
    };
    if response.status >= 400 {
        print_http_error(response.status, &response.body, stdout, stderr);
        return Err(TeamsError::Aborted);
    }
    match serde_json::from_slice::<Vec<ListedTeam>>(&response.body) {
        Ok(teams) => Ok(teams),
        Err(source) => Err(report_failure(stderr, &source)),
    }
}

fn display_teams(teams: &[ListedTeam], stdout: &mut dyn Write) {
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
                budget_text(team.max_budget),
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

fn budget_text(budget: Option<Decimal>) -> String {
    let Some(amount) = budget else {
        return String::from("Unlimited");
    };
    format!("${amount}")
}

fn report_failure(stderr: &mut dyn Write, failure: &dyn fmt::Display) -> TeamsError {
    let _wrote_failure = writeln!(stderr, "Error: {failure}");
    TeamsError::Aborted
}

mod tests;
