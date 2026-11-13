use std::io::Write;

use http::Method;
use serde::Deserialize;
use serde_json::value::RawValue;

use crate::commands::tables::write_json_response;
use crate::commands::{Client, Payload, Request};

use super::ModelsError;

#[derive(Deserialize)]
struct DataView {
    data: Box<RawValue>,
}

pub(super) fn fetch_data(
    client: &Client,
    path: &'static str,
) -> Result<Box<RawValue>, ModelsError> {
    let request = Request {
        method: Method::GET,
        url: client.resolve(path)?,
        headers: Vec::new(),
        payload: Payload::Empty,
    };
    let response = client.send(request)?;
    if response.status >= 400 {
        return Err(ModelsError::Http {
            status: response.status,
        });
    }
    let view: DataView = serde_json::from_slice(&response.body)
        .map_err(|source| ModelsError::Response { path, source })?;
    if !view.data.get().trim_start().starts_with('[') {
        return Err(ModelsError::ExpectedList { path });
    }
    Ok(view.data)
}

pub(super) fn post_json(
    client: &Client,
    path: &'static str,
    body: Vec<u8>,
) -> Result<Vec<u8>, ModelsError> {
    let request = Request {
        method: Method::POST,
        url: client.resolve(path)?,
        headers: Vec::new(),
        payload: Payload::Json(body),
    };
    let response = client.send(request)?;
    if response.status >= 400 {
        return Err(ModelsError::Http {
            status: response.status,
        });
    }
    Ok(response.body)
}

pub(super) fn write_json(
    path: &'static str,
    body: &[u8],
    stdout: &mut dyn Write,
) -> Result<(), ModelsError> {
    match write_json_response(body, stdout) {
        Ok(()) => Ok(()),
        Err(source) => Err(ModelsError::Response { path, source }),
    }
}
