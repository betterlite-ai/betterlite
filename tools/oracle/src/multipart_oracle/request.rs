use std::fs;
use std::path::{Component, Path};

use serde_json::Value;

use crate::OracleError;
use crate::case::{BodyKind, RequestBody, UpstreamAnswer};

use super::{BytesSpec, MAX_FIXTURE_BYTES, MAX_PARTS, MultipartSpec, PartSpec};

pub(crate) fn fixture(name: &str) -> Result<Vec<u8>, OracleError> {
    let path = Path::new(name);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(OracleError::Multipart(
            "fixture path escapes multipart fixture root",
        ));
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/multipart-oracle");
    let fixture_path = root.join(path);
    let bytes = fs::read(&fixture_path).map_err(|source| OracleError::Filesystem {
        path: fixture_path,
        source,
    })?;
    if bytes.len() > MAX_FIXTURE_BYTES {
        return Err(OracleError::Multipart("fixture exceeds 16777216 bytes"));
    }
    Ok(bytes)
}

pub(crate) fn request_body(body: &Value, kind: BodyKind) -> Result<RequestBody, OracleError> {
    match kind {
        BodyKind::Json => Ok(RequestBody::Json(
            serde_json::to_vec(body).map_err(OracleError::HttpBody)?,
        )),
        BodyKind::Multipart => {
            reject_unknown_fields(body, &["parts"])?;
            if let Some(parts) = body.get("parts").and_then(Value::as_array) {
                for part in parts {
                    reject_unknown_fields(
                        part,
                        &["name", "filename", "content_type", "text", "fixture"],
                    )?;
                }
            }
            let spec: MultipartSpec = serde_json::from_value(body.clone())
                .map_err(|error| OracleError::MultipartOwned(error.to_string()))?;
            encode_parts(&spec.parts).map(|(content_type, bytes)| RequestBody::Raw {
                content_type,
                bytes,
            })
        }
        BodyKind::Bytes => {
            reject_unknown_fields(body, &["body_kind", "fixture", "content_type"])?;
            let spec = decode_bytes_spec(body)?;
            Ok(RequestBody::Raw {
                content_type: spec.content_type,
                bytes: fixture(&spec.fixture)?,
            })
        }
    }
}

fn reject_unknown_fields(body: &Value, allowed: &[&str]) -> Result<(), OracleError> {
    let Some(fields) = body.as_object() else {
        return Ok(());
    };
    if let Some(field) = fields
        .keys()
        .find(|field| !allowed.iter().any(|allowed_field| allowed_field == field))
    {
        return Err(OracleError::MultipartOwned(format!(
            "field \"{field}\" is not allowed"
        )));
    }
    Ok(())
}

fn decode_bytes_spec(body: &Value) -> Result<BytesSpec, OracleError> {
    match serde_json::from_value(body.clone()) {
        Ok(spec) => Ok(spec),
        Err(error) => Err(OracleError::MultipartOwned(error.to_string())),
    }
}

pub(crate) fn answer_body(answer: &UpstreamAnswer) -> Result<Option<Vec<u8>>, OracleError> {
    if answer
        .headers
        .get("content-type")
        .is_some_and(|value| value.starts_with("application/octet-stream"))
    {
        let spec = decode_bytes_spec(&answer.body.clone().unwrap_or(Value::Null))?;
        if spec.body_kind.is_some_and(|kind| kind != BodyKind::Bytes) {
            return Err(OracleError::Multipart(
                "binary answer body_kind must be bytes",
            ));
        }
        Ok(Some(fixture(&spec.fixture)?))
    } else {
        Ok(None)
    }
}

pub(crate) fn encode_parts(parts: &[PartSpec]) -> Result<(String, Vec<u8>), OracleError> {
    if parts.len() > MAX_PARTS {
        return Err(OracleError::Multipart(
            "multipart contains more than 128 parts",
        ));
    }
    let boundary = "betterlite-oracle-boundary";
    let mut output = Vec::new();
    for part in parts {
        let payload = part_payload(part)?;
        output.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        output.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{}\"", part.name).as_bytes(),
        );
        if let Some(filename) = &part.filename {
            output.extend_from_slice(format!("; filename=\"{filename}\"").as_bytes());
        }
        output.extend_from_slice(b"\r\n");
        if let Some(content_type) = &part.content_type {
            output.extend_from_slice(format!("Content-Type: {content_type}\r\n").as_bytes());
        }
        output.extend_from_slice(b"\r\n");
        output.extend_from_slice(&payload);
        output.extend_from_slice(b"\r\n");
    }
    output.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Ok((format!("multipart/form-data; boundary={boundary}"), output))
}

fn part_payload(part: &PartSpec) -> Result<Vec<u8>, OracleError> {
    if part.name.is_empty() {
        return Err(OracleError::Multipart("part name must not be empty"));
    }
    match (&part.text, &part.fixture) {
        (Some(text), None) => Ok(text.as_bytes().to_vec()),
        (None, Some(name)) => fixture(name),
        _ => Err(OracleError::Multipart(
            "part must contain exactly one payload",
        )),
    }
}
