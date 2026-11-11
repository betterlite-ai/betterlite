use std::io::Write;

use http::Method;

use crate::commands::tables::write_json_response;

use super::{
    CREDENTIALS_PATH, Client, CredentialsError, DeleteCredential, JSON_HEADER, Payload, Request,
    send_or_abort,
};

pub(super) fn delete_credential(
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
    let response = send_or_abort(client, request, stdout, stderr)?;
    write_json_response(&response.body, stdout).map_err(|source| CredentialsError::Response {
        path: CREDENTIALS_PATH,
        source,
    })
}
