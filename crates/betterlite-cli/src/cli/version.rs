use std::io::Write;

use http::Method;
use serde::Deserialize;

use crate::cli_context::CliContext;
use crate::commands::{Client, ClientError, Payload, Request};

use super::Failure;

pub(crate) const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");
const READINESS_PATH: &str = "health/readiness";

#[derive(Deserialize)]
struct Readiness {
    #[serde(default)]
    litellm_version: Option<String>,
}

#[derive(Debug, thiserror::Error)]
enum ServerVersionError {
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("the proxy answered HTTP {status}")]
    Status { status: u16 },
    #[error("the readiness response is not valid JSON")]
    Json(#[source] serde_json::Error),
}

pub(crate) fn print_version(
    context: &CliContext,
    client: &Client,
    stdout: &mut dyn Write,
) -> Result<(), Failure> {
    writeln!(stdout, "betterlite Proxy CLI Version: {CLI_VERSION}")?;
    if !context.base_url.is_empty() {
        writeln!(stdout, "betterlite Proxy Server URL: {}", context.base_url)?;
    }
    match server_version(client) {
        Ok(Some(version)) => writeln!(stdout, "betterlite Proxy Server Version: {version}")?,
        Ok(None) => writeln!(stdout, "betterlite Proxy Server Version: (unavailable)")?,
        Err(error) => writeln!(stdout, "Could not retrieve server version: {error}")?,
    }
    Ok(())
}

fn server_version(client: &Client) -> Result<Option<String>, ServerVersionError> {
    let url = client.resolve(READINESS_PATH)?;
    let response = client.send(Request {
        method: Method::GET,
        url,
        headers: Vec::new(),
        payload: Payload::Empty,
    })?;
    if !(200..300).contains(&response.status) {
        return Err(ServerVersionError::Status {
            status: response.status,
        });
    }
    let readiness =
        serde_json::from_slice::<Readiness>(&response.body).map_err(ServerVersionError::Json)?;
    Ok(readiness.litellm_version)
}
