use std::io::Write;

use clap::{Args, Subcommand};
use http::Method;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use super::{
    Client, ClientError, OutputFormat, Payload, Request, print_http_error, write_json_response,
    write_table,
};

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
    #[error("{message}")]
    Usage { message: String },
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
    #[serde(default)]
    credentials: Vec<CredentialItem>,
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

#[derive(Serialize)]
struct CreateBody<'request> {
    #[serde(rename = "credential_name")]
    name: &'request str,
    #[serde(rename = "credential_info")]
    info: &'request RawValue,
    #[serde(rename = "credential_values")]
    values: &'request RawValue,
}

pub fn run_credentials_command(
    command: CredentialsCommand,
    client: &Client,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), CredentialsError> {
    match command {
        CredentialsCommand::List(options) => list_credentials(&options, client, stdout),
        CredentialsCommand::Create(options) => create_credential(&options, client, stdout, stderr),
        CredentialsCommand::Delete(options) => delete_credential(&options, client, stdout, stderr),
        CredentialsCommand::Get(options) => get_credential(&options, client, stdout),
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
    let response = client.send(request)?;
    if response.status >= 400 {
        return Err(CredentialsError::Http {
            status: response.status,
        });
    }
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

fn create_credential(
    options: &CreateCredential,
    client: &Client,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), CredentialsError> {
    let info: &RawValue =
        serde_json::from_str(&options.info).map_err(|source| CredentialsError::Usage {
            message: format!("Invalid JSON: {source}"),
        })?;
    let values: &RawValue =
        serde_json::from_str(&options.values).map_err(|source| CredentialsError::Usage {
            message: format!("Invalid JSON: {source}"),
        })?;
    let body = serde_json::to_vec(&CreateBody {
        name: &options.credential_name,
        info,
        values,
    })
    .map_err(CredentialsError::Encode)?;
    let request = Request {
        method: Method::POST,
        url: client.resolve(CREDENTIALS_PATH)?,
        headers: Vec::new(),
        payload: Payload::Json(body),
    };
    let response = client.send(request)?;
    if response.status >= 400 {
        print_http_error(response.status, &response.body, stdout, stderr);
        return Err(CredentialsError::Aborted);
    }
    write_json_response(&response.body, stdout).map_err(|source| CredentialsError::Response {
        path: CREDENTIALS_PATH,
        source,
    })
}

fn delete_credential(
    options: &DeleteCredential,
    client: &Client,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), CredentialsError> {
    let mut url = client.resolve(CREDENTIALS_PATH)?;
    url.path_segments_mut()
        .map_err(|()| CredentialsError::Uri {
            uri: CREDENTIALS_PATH,
        })?
        .push(&options.credential_name);
    let request = Request {
        method: Method::DELETE,
        url,
        headers: vec![(String::from(JSON_HEADER.0), String::from(JSON_HEADER.1))],
        payload: Payload::Empty,
    };
    let response = client.send(request)?;
    if response.status >= 400 {
        print_http_error(response.status, &response.body, stdout, stderr);
        return Err(CredentialsError::Aborted);
    }
    write_json_response(&response.body, stdout).map_err(|source| CredentialsError::Response {
        path: CREDENTIALS_PATH,
        source,
    })
}

fn get_credential(
    options: &GetCredential,
    client: &Client,
    stdout: &mut dyn Write,
) -> Result<(), CredentialsError> {
    let mut url = client.resolve(CREDENTIALS_PATH)?;
    url.path_segments_mut()
        .map_err(|()| CredentialsError::Uri {
            uri: CREDENTIALS_PATH,
        })?
        .extend(["by_name", options.credential_name.as_str()]);
    let request = Request {
        method: Method::GET,
        url,
        headers: vec![(String::from(JSON_HEADER.0), String::from(JSON_HEADER.1))],
        payload: Payload::Empty,
    };
    let response = client.send(request)?;
    if response.status >= 400 {
        return Err(CredentialsError::Http {
            status: response.status,
        });
    }
    write_json_response(&response.body, stdout).map_err(|source| CredentialsError::Response {
        path: CREDENTIALS_PATH,
        source,
    })
}

mod tests;
