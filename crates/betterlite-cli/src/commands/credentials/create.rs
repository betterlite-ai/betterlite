use std::io::Write;

use http::Method;
use serde::Serialize;
use serde_json::value::RawValue;

use crate::commands::tables::write_json_response;

use super::{
    CREDENTIALS_PATH, Client, CreateCredential, CredentialsError, Payload, Request, send_or_abort,
};

#[derive(Serialize)]
struct CreateBody<'request> {
    #[serde(rename = "credential_name")]
    name: &'request str,
    #[serde(rename = "credential_info")]
    info: &'request RawValue,
    #[serde(rename = "credential_values")]
    values: &'request RawValue,
}

pub(super) fn create_credential(
    options: &CreateCredential,
    client: &Client,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), CredentialsError> {
    let info: &RawValue = serde_json::from_str(&options.info)
        .map_err(|source| CredentialsError::InvalidJson { source })?;
    let values: &RawValue = serde_json::from_str(&options.values)
        .map_err(|source| CredentialsError::InvalidJson { source })?;
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
    let response = send_or_abort(client, request, stdout, stderr)?;
    write_json_response(&response.body, stdout).map_err(|source| CredentialsError::Response {
        path: CREDENTIALS_PATH,
        source,
    })
}
