use std::io::Write;

use clap::{Args, Subcommand};
use http::Method;
use serde::Deserialize;

use crate::pkce_login::HttpResponse;

use super::tables::{OutputFormat, print_http_error, write_json_response, write_table};
use super::{Client, ClientError, Payload, Request};

mod create;
mod delete;
mod get;

const CREDENTIALS_PATH: &str = "/credentials";
const JSON_HEADER: (&str, &str) = ("content-type", "application/json");

#[derive(Debug, Subcommand)]
#[command(about = "Manage credentials for the LiteLLM proxy server")]
pub enum CredentialsCommand {
    List(ListCredentials),
    Create(CreateCredential),
    Delete(DeleteCredential),
    Get(GetCredential),
}

#[derive(Debug, Args)]
pub struct ListCredentials {
    #[arg(
        long,
        value_enum,
        default_value_t = OutputFormat::Table,
        help = "Output format (table or json)"
    )]
    format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct CreateCredential {
    credential_name: String,
    #[arg(long, help = "JSON string containing credential info")]
    info: String,
    #[arg(long, help = "JSON string containing credential values")]
    values: String,
}

#[derive(Debug, Args)]
pub struct DeleteCredential {
    credential_name: String,
}

#[derive(Debug, Args)]
pub struct GetCredential {
    credential_name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialsError {
    #[error("the command was aborted")]
    Aborted,
    #[error("Invalid JSON: {source}")]
    InvalidJson {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("the proxy answered HTTP {status}")]
    Http { status: u16 },
    #[error("{uri} cannot be addressed")]
    Uri { uri: &'static str },
    #[error("unexpected response from {path}")]
    Response {
        path: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("cannot encode the credential request")]
    Encode(#[source] serde_json::Error),
}

#[derive(Debug, Deserialize)]
struct CredentialsView {
    credentials: Option<Vec<CredentialItem>>,
}

#[derive(Debug, Deserialize)]
struct CredentialItem {
    #[serde(default)]
    credential_name: Option<String>,
    #[serde(default)]
    credential_info: Option<CredentialInfo>,
}

#[derive(Debug, Deserialize)]
struct CredentialInfo {
    #[serde(default)]
    custom_llm_provider: Option<String>,
}

pub fn run_credentials_command(
    command: CredentialsCommand,
    client: &Client,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), CredentialsError> {
    match command {
        CredentialsCommand::List(options) => list_credentials(&options, client, stdout),
        CredentialsCommand::Create(options) => {
            create::create_credential(&options, client, stdout, stderr)
        }
        CredentialsCommand::Delete(options) => {
            delete::delete_credential(&options, client, stdout, stderr)
        }
        CredentialsCommand::Get(options) => get::get_credential(&options, client, stdout),
    }
}

fn list_credentials(
    options: &ListCredentials,
    client: &Client,
    stdout: &mut dyn Write,
) -> Result<(), CredentialsError> {
    let request = Request {
        method: Method::GET,
        url: client.resolve(CREDENTIALS_PATH)?,
        headers: vec![(String::from(JSON_HEADER.0), String::from(JSON_HEADER.1))],
        payload: Payload::Empty,
    };
    let response = send_or_http_error(client, request)?;
    if options.format == OutputFormat::Json {
        return write_json_response(&response.body, stdout).map_err(|source| {
            CredentialsError::Response {
                path: CREDENTIALS_PATH,
                source,
            }
        });
    }
    let view: CredentialsView =
        serde_json::from_slice(&response.body).map_err(|source| CredentialsError::Response {
            path: CREDENTIALS_PATH,
            source,
        })?;
    write_table(
        "Credentials",
        &["Credential Name", "Custom LLM Provider"],
        credential_rows(view),
        stdout,
    );
    Ok(())
}

fn credential_rows(view: CredentialsView) -> Vec<Vec<String>> {
    view.credentials
        .unwrap_or_default()
        .into_iter()
        .map(|credential| {
            vec![
                credential.credential_name.unwrap_or_default(),
                credential
                    .credential_info
                    .and_then(|info| info.custom_llm_provider)
                    .unwrap_or_default(),
            ]
        })
        .collect()
}

fn send_or_abort(
    client: &Client,
    request: Request,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<HttpResponse, CredentialsError> {
    let response = client.send(request)?;
    if response.status >= 400 {
        print_http_error(response.status, &response.body, stdout, stderr);
        return Err(CredentialsError::Aborted);
    }
    Ok(response)
}

fn send_or_http_error(client: &Client, request: Request) -> Result<HttpResponse, CredentialsError> {
    let response = client.send(request)?;
    if response.status >= 400 {
        return Err(CredentialsError::Http {
            status: response.status,
        });
    }
    Ok(response)
}

mod tests;
