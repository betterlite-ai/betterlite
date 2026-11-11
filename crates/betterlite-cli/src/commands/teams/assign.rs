use std::io::Write;

use http::Method;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

use crate::commands::tables::print_http_error;
use crate::commands::{Client, Payload, Request};

use super::{TeamsError, report_failure};

const UPDATE_PATH: &str = "/key/update";

#[derive(Serialize)]
struct KeyUpdateBody<'request> {
    key: &'request str,
    team_id: &'request str,
}

pub(super) fn update_key(
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
