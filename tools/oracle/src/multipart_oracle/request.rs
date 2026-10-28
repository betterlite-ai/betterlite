use std::fs::File;
use std::io::Read as _;
use std::path::{Component, Path};

use serde::Deserialize as _;
use serde_json::Value;

use crate::case::{BodyKind, RequestBody, UpstreamAnswer};

use super::error::MultipartError;
use super::{BytesAnswer, BytesSpec, LIMITS, MultipartSpec, PartSpec};

pub(crate) fn fixture(name: &str) -> Result<Vec<u8>, MultipartError> {
    let path = Path::new(name);
    if path.has_root()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(MultipartError::FixtureEscape);
    }
    read_fixture(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/multipart-oracle")
            .join(path),
    )
}

pub(crate) fn read_fixture(path: &Path) -> Result<Vec<u8>, MultipartError> {
    let read_error = |source| MultipartError::FixtureRead {
        path: path.to_owned(),
        source,
    };
    let too_large = MultipartError::FixtureTooLarge {
        limit: LIMITS.fixture_bytes,
    };
    let readable = u64::try_from(LIMITS.fixture_bytes.saturating_add(1)).map_err(|_source| {
        MultipartError::FixtureTooLarge {
            limit: LIMITS.fixture_bytes,
        }
    })?;
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(read_error)?
        .take(readable)
        .read_to_end(&mut bytes)
        .map_err(read_error)?;
    if bytes.len() > LIMITS.fixture_bytes {
        return Err(too_large);
    }
    Ok(bytes)
}

pub(crate) fn request_body(body: &Value, kind: BodyKind) -> Result<RequestBody, MultipartError> {
    match kind {
        BodyKind::Json if body.is_null() => Ok(RequestBody::Empty),
        BodyKind::Json => Ok(RequestBody::Json(
            serde_json::to_vec(body).map_err(MultipartError::Specification)?,
        )),
        BodyKind::Multipart => {
            reject_unknown_fields(body, &["parts"])?;
            for part in body
                .get("parts")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                reject_unknown_fields(
                    part,
                    &["name", "filename", "content_type", "text", "fixture"],
                )?;
            }
            let spec = MultipartSpec::deserialize(body).map_err(MultipartError::Specification)?;
            let (content_type, bytes) = encode_parts(&spec.parts)?;
            Ok(RequestBody::Raw {
                content_type,
                bytes,
            })
        }
        BodyKind::Bytes => {
            reject_unknown_fields(body, &["fixture", "content_type"])?;
            let spec = BytesSpec::deserialize(body).map_err(MultipartError::Specification)?;
            Ok(RequestBody::Raw {
                content_type: spec.content_type,
                bytes: fixture(&spec.fixture)?,
            })
        }
    }
}

fn reject_unknown_fields(body: &Value, allowed: &[&str]) -> Result<(), MultipartError> {
    let unknown = body
        .as_object()
        .into_iter()
        .flat_map(|fields| fields.keys())
        .find(|field| !allowed.contains(&field.as_str()));
    unknown.map_or(Ok(()), |field| {
        Err(MultipartError::UnknownField {
            field: field.clone(),
        })
    })
}

pub(crate) fn answer_body(
    answer: &UpstreamAnswer,
) -> Result<Option<(String, Vec<u8>)>, MultipartError> {
    let Some(body) = answer
        .body
        .as_ref()
        .filter(|body| body.get("body_kind").is_some())
    else {
        return Ok(None);
    };
    reject_unknown_fields(body, &["body_kind", "fixture", "content_type"])?;
    let spec = BytesAnswer::deserialize(body).map_err(MultipartError::Specification)?;
    if spec.body_kind != BodyKind::Bytes {
        return Err(MultipartError::AnswerKind);
    }
    if let Some(header) = answer.headers.get("content-type")
        && *header != spec.content_type
    {
        return Err(MultipartError::AnswerContentType {
            declared: spec.content_type,
            header: header.clone(),
        });
    }
    Ok(Some((spec.content_type, fixture(&spec.fixture)?)))
}

pub(crate) fn encode_parts(parts: &[PartSpec]) -> Result<(String, Vec<u8>), MultipartError> {
    if parts.len() > LIMITS.parts {
        return Err(MultipartError::TooManyParts {
            limit: LIMITS.parts,
        });
    }
    let payloads = parts
        .iter()
        .map(part_payload)
        .collect::<Result<Vec<_>, _>>()?;
    // each payload byte starts at most one candidate, so one more candidate than bytes always finds a free boundary
    let candidates = payloads.iter().map(Vec::len).sum::<usize>();
    let boundary = (0..=candidates)
        .map(|attempt| format!("betterlite-oracle-{attempt}"))
        .find(|boundary| {
            payloads.iter().all(|payload| {
                !payload
                    .windows(boundary.len())
                    .any(|window| window == boundary.as_bytes())
            })
        })
        .ok_or(MultipartError::Boundary)?;
    let mut output = Vec::new();
    for (part, payload) in parts.iter().zip(payloads) {
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

fn part_payload(part: &PartSpec) -> Result<Vec<u8>, MultipartError> {
    if part.name.is_empty() {
        return Err(MultipartError::PartName);
    }
    let quoted = [Some(&part.name), part.filename.as_ref()];
    if quoted
        .into_iter()
        .flatten()
        .any(|value| value.contains(['"', '\r', '\n']))
        || part
            .content_type
            .as_ref()
            .is_some_and(|value| value.contains(['\r', '\n']))
    {
        return Err(MultipartError::PartMetadata);
    }
    match (&part.text, &part.fixture) {
        (Some(text), None) => Ok(text.as_bytes().to_vec()),
        (None, Some(name)) => fixture(name),
        (Some(_), Some(_)) | (None, None) => Err(MultipartError::PartPayload),
    }
}
