use std::fmt;
use std::io::Write;

use clap::{Args, Subcommand};
use http::Method;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::credentials::contracts::CliTeam;
use crate::credentials::notice::MessageSink;
use crate::pkce_login::HttpResponse;
use crate::sso_polling::selection::prompt_team_selection;
use crate::sso_polling::terminal::SelectionInput;

use super::tables::print_http_error;
use super::{Client, Payload, Request};

mod assign;
mod display;

use assign::update_key;
use display::display_teams;

const LIST_PATH: &str = "/team/list";
const AVAILABLE_PATH: &str = "/team/available";
const NO_TEAMS: &str = "No teams found for your user.";
const NO_AVAILABLE_TEAMS: &str = "No available teams to join.";
const NO_API_KEY: &str = "No API key found. Please login first using 'lite login'";
const OPERATION_CANCELLED: &str = "Operation cancelled.";
const AUTHENTICATION_FAILED: &str = "Authentication failed. Check your API key.";

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
    let teams = fetch_teams(client, stdout, stderr)?;
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
    let response = send_list_request(client, AVAILABLE_PATH, stderr)?;
    if response.status == 401 {
        return Err(report_failure(stderr, &AUTHENTICATION_FAILED));
    }
    if response.status >= 400 {
        print_http_error(response.status, &response.body, stdout, stderr);
        return Ok(());
    }
    let teams = parse_teams(&response, stderr)?;
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
        .into_iter()
        .map(|team| CliTeam {
            team_id: team.team_id,
            team_alias: team.team_alias,
            models: team.models.unwrap_or_default(),
            max_budget: team.max_budget,
        })
        .collect();
    let Some(selected) = prompt_team_selection(&prompt_teams, terminal, echo) else {
        let _wrote_cancelled = writeln!(stdout, "{OPERATION_CANCELLED}");
        return Ok(None);
    };
    Ok(selected.team_id)
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
    let response = send_list_request(client, LIST_PATH, stderr)?;
    if response.status == 401 {
        return Err(report_failure(stderr, &AUTHENTICATION_FAILED));
    }
    if response.status >= 400 {
        print_http_error(response.status, &response.body, stdout, stderr);
        return Err(TeamsError::Aborted);
    }
    parse_teams(&response, stderr)
}

fn send_list_request(
    client: &Client,
    path: &str,
    stderr: &mut dyn Write,
) -> Result<HttpResponse, TeamsError> {
    let Ok(url) = client.resolve(path) else {
        return Err(report_failure(stderr, &path));
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
    match client.send(request) {
        Ok(response) => Ok(response),
        Err(source) => Err(report_failure(stderr, &source)),
    }
}

fn parse_teams(
    response: &HttpResponse,
    stderr: &mut dyn Write,
) -> Result<Vec<ListedTeam>, TeamsError> {
    match serde_json::from_slice::<Vec<ListedTeam>>(&response.body) {
        Ok(teams) => Ok(teams),
        Err(source) => Err(report_failure(stderr, &source)),
    }
}

fn report_failure(stderr: &mut dyn Write, failure: &dyn fmt::Display) -> TeamsError {
    let _wrote_failure = writeln!(stderr, "Error: {failure}");
    TeamsError::Aborted
}

mod tests;
