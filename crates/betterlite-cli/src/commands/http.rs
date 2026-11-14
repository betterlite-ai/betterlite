use std::io::Write;

use clap::{Args, Subcommand};
use http::Method;
use http::method::InvalidMethod;

use crate::cli_context::CliContext;

use super::tables::print_http_error;
use super::{Client, ClientError, Request};

mod json_body;
mod tests;

#[derive(Debug, Subcommand)]
#[command(about = "Make HTTP requests to the LiteLLM proxy server")]
pub enum HttpCommand {
    Request(RequestArgs),
}

#[derive(Debug, Args)]
pub struct RequestArgs {
    #[arg(help = "HTTP method (GET, POST, PUT, DELETE, etc.)")]
    method: String,
    #[arg(help = "URI path (will be appended to base_url)")]
    uri: String,
    #[arg(
        short,
        long,
        help = "Data to send in the request body (as JSON string)"
    )]
    data: Option<String>,
    #[arg(
        short,
        long,
        help = "JSON data to send in the request body (as JSON string)"
    )]
    json: Option<String>,
    #[arg(
        short = 'H',
        long = "header",
        help = "HTTP headers in 'key:value' format. Can be specified multiple times."
    )]
    header: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("the command was aborted")]
    Aborted,
    #[error("Invalid header format: {header}. Expected format: 'key:value'")]
    HeaderFormat { header: String },
    #[error("Invalid JSON format: {source}")]
    InvalidJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("{method} is not a valid HTTP method")]
    Method {
        method: String,
        #[source]
        source: InvalidMethod,
    },
    #[error("the response body is not valid JSON")]
    Response {
        #[source]
        source: serde_json::Error,
    },
    #[error("the request body could not be encoded")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

pub fn run_http_command(
    command: HttpCommand,
    _cli_context: &CliContext,
    client: &Client,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), HttpError> {
    match command {
        HttpCommand::Request(options) => send_request(&options, client, stdout, stderr),
    }
}

fn send_request(
    options: &RequestArgs,
    client: &Client,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), HttpError> {
    let method =
        Method::from_bytes(options.method.to_uppercase().as_bytes()).map_err(|source| {
            HttpError::Method {
                method: options.method.clone(),
                source,
            }
        })?;
    let headers = parse_headers(&options.header)?;
    let keep_content_type = headers
        .iter()
        .any(|(name, _value)| name.eq_ignore_ascii_case("content-type"));
    let payload = json_body::request_payload(
        options.json.as_deref(),
        options.data.as_deref(),
        keep_content_type,
    )?;
    let request = Request {
        method,
        url: client.resolve(&options.uri)?,
        headers,
        payload,
    };
    let response = client.send(request)?;
    if response.status >= 400 {
        print_http_error(response.status, &response.body, stdout, stderr);
        return Err(HttpError::Aborted);
    }
    json_body::write_pretty(&response.body, stdout)
}

fn parse_headers(entries: &[String]) -> Result<Vec<(String, String)>, HttpError> {
    let mut headers = Vec::new();
    for entry in entries {
        let Some((name, value)) = entry.split_once(':') else {
            return Err(HttpError::HeaderFormat {
                header: entry.clone(),
            });
        };
        headers.push((name.trim().to_owned(), value.trim().to_owned()));
    }
    Ok(headers)
}
