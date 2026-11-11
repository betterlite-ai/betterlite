use std::io::Write;

use http::Method;

use crate::commands::tables::write_json_response;

use super::{
    CREDENTIALS_PATH, Client, CredentialsError, GetCredential, JSON_HEADER, Payload, Request,
    send_or_http_error,
};

pub(super) fn get_credential(
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
    let response = send_or_http_error(client, request)?;
    write_json_response(&response.body, stdout).map_err(|source| CredentialsError::Response {
        path: CREDENTIALS_PATH,
        source,
    })
}
