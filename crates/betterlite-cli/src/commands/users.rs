use std::io::Write;

use clap::{Args, Subcommand};
use http::Method;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::{Client, ClientError, Payload, Request, write_json_response, write_table};

const LIST_PATH: &str = "/user/list";
const INFO_PATH: &str = "/user/info";
const NEW_PATH: &str = "/user/new";
const DELETE_PATH: &str = "/user/delete";
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

#[derive(Serialize)]
struct CreateUserBody<'request> {
    user_email: &'request str,
    user_role: &'request str,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_alias: Option<&'request str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    teams: Option<&'request [String]>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    max_budget: Option<Decimal>,
}

#[derive(Serialize)]
struct DeleteUsersBody<'request> {
    user_ids: &'request [String],
}

pub fn run_users_command(
    command: UsersCommand,
    client: &Client,
    stdout: &mut dyn Write,
) -> Result<(), UsersError> {
    match command {
        UsersCommand::List(options) => list_users(&options, client, stdout),
        UsersCommand::Get(options) => get_user(&options, client, stdout),
        UsersCommand::Create(options) => create_user(&options, client, stdout),
        UsersCommand::Delete(options) => delete_user(&options, client, stdout),
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
    let rows = users
        .into_iter()
        .map(|user| {
            vec![
                user.user_id.unwrap_or_default(),
                user.user_email.unwrap_or_default(),
                user.user_role.unwrap_or_default(),
                user.teams.unwrap_or_default().join(", "),
            ]
        })
        .collect();
    write_table(
        "Users",
        &["User ID", "Email", "Role", "Teams"],
        rows,
        stdout,
    );
    Ok(())
}

fn get_user(options: &GetUser, client: &Client, stdout: &mut dyn Write) -> Result<(), UsersError> {
    let mut url = client.resolve(INFO_PATH)?;
    if let Some(user_id) = &options.user_id {
        url.query_pairs_mut().append_pair("user_id", user_id);
    }
    let request = Request {
        method: Method::GET,
        url,
        headers: vec![(String::from(JSON_HEADER.0), String::from(JSON_HEADER.1))],
        payload: Payload::Empty,
    };
    let response = client.send(request)?;
    if response.status >= 400 {
        return Err(UsersError::Http {
            status: response.status,
        });
    }
    write_json_response(&response.body, stdout).map_err(|source| UsersError::Response {
        path: INFO_PATH,
        source,
    })
}

fn create_user(
    options: &CreateUser,
    client: &Client,
    stdout: &mut dyn Write,
) -> Result<(), UsersError> {
    let teams = (!options.team.is_empty()).then_some(options.team.as_slice());
    let body = serde_json::to_vec(&CreateUserBody {
        user_email: &options.email,
        user_role: &options.role,
        user_alias: options.alias.as_deref(),
        teams,
        max_budget: options.max_budget,
    })
    .map_err(UsersError::Encode)?;
    let request = Request {
        method: Method::POST,
        url: client.resolve(NEW_PATH)?,
        headers: Vec::new(),
        payload: Payload::Json(body),
    };
    let response = client.send(request)?;
    if response.status >= 400 {
        return Err(UsersError::Http {
            status: response.status,
        });
    }
    write_json_response(&response.body, stdout).map_err(|source| UsersError::Response {
        path: NEW_PATH,
        source,
    })
}

fn delete_user(
    options: &DeleteUser,
    client: &Client,
    stdout: &mut dyn Write,
) -> Result<(), UsersError> {
    let body = serde_json::to_vec(&DeleteUsersBody {
        user_ids: &options.user_ids,
    })
    .map_err(UsersError::Encode)?;
    let request = Request {
        method: Method::POST,
        url: client.resolve(DELETE_PATH)?,
        headers: Vec::new(),
        payload: Payload::Json(body),
    };
    let response = client.send(request)?;
    if response.status >= 400 {
        return Err(UsersError::Http {
            status: response.status,
        });
    }
    write_json_response(&response.body, stdout).map_err(|source| UsersError::Response {
        path: DELETE_PATH,
        source,
    })
}

mod tests;
