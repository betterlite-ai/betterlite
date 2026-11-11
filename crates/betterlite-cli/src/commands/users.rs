use std::io::Write;

use clap::{Args, Subcommand};
use http::Method;
use rust_decimal::Decimal;
use serde::Deserialize;

use super::tables::{write_json_response, write_table};
use super::{Client, ClientError, Payload, Request};

mod create;
mod delete;
mod get;

const LIST_PATH: &str = "/user/list";
const JSON_HEADER: (&str, &str) = ("content-type", "application/json");

#[derive(Debug, Subcommand)]
#[command(about = "Manage users on your LiteLLM proxy server")]
pub enum UsersCommand {
    List(ListUsers),
    Get(GetUser),
    Create(CreateUser),
    Delete(DeleteUser),
}

#[derive(Debug, Args)]
pub struct ListUsers;

#[derive(Debug, Args)]
pub struct GetUser {
    #[arg(long = "id", help = "ID of the user to retrieve")]
    user_id: Option<String>,
}

#[derive(Debug, Args)]
pub struct CreateUser {
    #[arg(long, help = "User email")]
    email: String,
    #[arg(long, default_value = "internal_user", help = "User role")]
    role: String,
    #[arg(long, help = "User alias")]
    alias: Option<String>,
    #[arg(long = "team", help = "Team IDs (can specify multiple)")]
    team: Vec<String>,
    #[arg(long = "max-budget", help = "Max budget for user")]
    max_budget: Option<Decimal>,
}

#[derive(Debug, Args)]
pub struct DeleteUser {
    user_ids: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum UsersError {
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("the proxy answered HTTP {status}")]
    Http { status: u16 },
    #[error("unexpected response from {path}")]
    Response {
        path: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("cannot encode the user request")]
    Encode(#[source] serde_json::Error),
}

#[derive(Debug, Deserialize)]
struct UsersView {
    #[serde(deserialize_with = "Option::deserialize")]
    users: Option<Vec<UserItem>>,
}

#[derive(Debug, Deserialize)]
struct UserItem {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    user_email: Option<String>,
    #[serde(default)]
    user_role: Option<String>,
    #[serde(default)]
    teams: Option<Vec<String>>,
}

pub fn run_users_command(
    command: UsersCommand,
    client: &Client,
    stdout: &mut dyn Write,
) -> Result<(), UsersError> {
    match command {
        UsersCommand::List(options) => list_users(&options, client, stdout),
        UsersCommand::Get(options) => get::get_user(&options, client, stdout),
        UsersCommand::Create(options) => create::create_user(&options, client, stdout),
        UsersCommand::Delete(options) => delete::delete_user(&options, client, stdout),
    }
}

fn list_users(
    _options: &ListUsers,
    client: &Client,
    stdout: &mut dyn Write,
) -> Result<(), UsersError> {
    let request = Request {
        method: Method::GET,
        url: client.resolve(LIST_PATH)?,
        headers: vec![(String::from(JSON_HEADER.0), String::from(JSON_HEADER.1))],
        payload: Payload::Empty,
    };
    let response = client.send(request)?;
    if response.status >= 400 {
        return Err(UsersError::Http {
            status: response.status,
        });
    }
    let view: UsersView =
        serde_json::from_slice(&response.body).map_err(|source| UsersError::Response {
            path: LIST_PATH,
            source,
        })?;
    let Some(users) = view.users.filter(|users| !users.is_empty()) else {
        let _unused = writeln!(stdout, "No users found.");
        return Ok(());
    };
    write_table(
        "Users",
        &["User ID", "Email", "Role", "Teams"],
        user_rows(users),
        stdout,
    );
    Ok(())
}

fn user_rows(users: Vec<UserItem>) -> Vec<Vec<String>> {
    users
        .into_iter()
        .map(|user| {
            vec![
                user.user_id.unwrap_or_default(),
                user.user_email.unwrap_or_default(),
                user.user_role.unwrap_or_default(),
                user.teams.unwrap_or_default().join(", "),
            ]
        })
        .collect()
}

fn post_user(
    client: &Client,
    path: &'static str,
    body: Vec<u8>,
    stdout: &mut dyn Write,
) -> Result<(), UsersError> {
    let request = Request {
        method: Method::POST,
        url: client.resolve(path)?,
        headers: Vec::new(),
        payload: Payload::Json(body),
    };
    send_and_write(client, request, path, stdout)
}

fn send_and_write(
    client: &Client,
    request: Request,
    path: &'static str,
    stdout: &mut dyn Write,
) -> Result<(), UsersError> {
    let response = client.send(request)?;
    if response.status >= 400 {
        return Err(UsersError::Http {
            status: response.status,
        });
    }
    write_json_response(&response.body, stdout)
        .map_err(|source| UsersError::Response { path, source })
}

mod tests;
