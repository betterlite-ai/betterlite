use std::collections::BTreeMap;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_config::sections::guardrail_params::decode::decode_patch_request;
use betterlite_config::sections::guardrail_params::error::GuardrailParamsError;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::Guardrail;
use betterlite_config::sections::guardrail_params::routes::PatchGuardrailRequest;
use betterlite_db::budget_permission_rows::JsonData;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde::Serialize;
use serde_json::error::Category;
use serde_json::value::RawValue;
use serde_path_to_error::Path;
use serde_path_to_error::Segment;
use thiserror::Error;

const JSON_DECODE_ERROR: &str = "JSON decode error";
const FIELD_REQUIRED: &str = "Field required";
const BODY: &str = "body";
const GUARDRAIL: &str = "guardrail";
const LITELLM_PARAMS: &str = "litellm_params";
const GUARDRAIL_INFO: &str = "guardrail_info";

pub struct WriteRequest {
    pub guardrail: Guardrail,
    pub guardrail_info: Option<Box<RawValue>>,
}

#[derive(Deserialize)]
struct WriteBody {
    guardrail: Box<RawValue>,
}

#[derive(Deserialize)]
struct RawGuardrail {
    guardrail_name: String,
    litellm_params: Box<RawValue>,
    guardrail_info: Option<Box<RawValue>>,
    guardrail_id: Option<String>,
    policy_template: Option<String>,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Error, Serialize)]
#[error("{msg}")]
pub struct BodyError {
    #[serde(rename = "type")]
    issue_type: &'static str,
    loc: Vec<Location>,
    msg: String,
    input: JsonData,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum Location {
    Field(String),
    Position(usize),
}

#[derive(Serialize)]
struct ValidationBody {
    detail: [BodyError; 1],
}

pub fn decode_write_request(bytes: &[u8]) -> Result<WriteRequest, BodyError> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let body: WriteBody = serde_path_to_error::deserialize(&mut deserializer)
        .map_err(|source| BodyError::serde_at(bytes, &[], &source))?;
    deserializer
        .end()
        .map_err(|_trailing| BodyError::invalid_json())?;
    let text = body.guardrail.get();
    let raw = decode_raw_guardrail(bytes, text)?;
    let guardrail_info = match raw.guardrail_info {
        Some(info) => Some(info),
        None if sent(text, GUARDRAIL_INFO) => Some(RawValue::NULL.to_owned()),
        None => None,
    };
    let params = decode_params(bytes, raw.litellm_params.get())?;
    let info = match &guardrail_info {
        Some(info) => decode_info(bytes, info.get())?,
        None => None,
    };
    Ok(WriteRequest {
        guardrail: Guardrail {
            guardrail_id: raw.guardrail_id,
            guardrail_name: raw.guardrail_name,
            litellm_params: params,
            guardrail_info: info,
            policy_template: raw.policy_template,
            created_at: raw.created_at,
            updated_at: raw.updated_at,
            team_id: None,
        },
        guardrail_info,
    })
}

pub fn decode_patch_body(bytes: &[u8]) -> Result<PatchGuardrailRequest, BodyError> {
    match decode_patch_request(bytes) {
        Ok(request) => Ok(request),
        Err(GuardrailParamsError::TrailingData) => Err(BodyError::invalid_json()),
        Err(source) => {
            if serde_json::from_slice::<JsonData>(bytes).is_err() {
                return Err(BodyError::invalid_json());
            }
            Err(BodyError::params(bytes, &[], &source))
        }
    }
}

fn decode_raw_guardrail(bytes: &[u8], text: &str) -> Result<RawGuardrail, BodyError> {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    serde_path_to_error::deserialize(&mut deserializer)
        .map_err(|source| BodyError::serde_at(bytes, &[GUARDRAIL], &source))
}

// The typed decode turns a null guardrail_info into None, so the key decides whether it was sent
fn sent(text: &str, key: &str) -> bool {
    serde_json::from_str::<BTreeMap<String, JsonData>>(text)
        .is_ok_and(|fields| fields.contains_key(key))
}

fn decode_info(bytes: &[u8], text: &str) -> Result<Option<OpaqueParams>, BodyError> {
    serde_json::from_str::<Option<OpaqueParams>>(text)
        .map_err(|source| BodyError::info(bytes, &source))
}

// decode_litellm_params flattens every serde try_from error into a field path, so the typed convert keeps the missing and rule variants
fn decode_params(bytes: &[u8], text: &str) -> Result<LitellmParams, BodyError> {
    let map = match serde_json::from_str::<OpaqueParams>(text) {
        Ok(map) => map,
        Err(source) => {
            let loc = body_location(&[GUARDRAIL, LITELLM_PARAMS]);
            return Err(BodyError {
                issue_type: "value_error",
                input: value_at(bytes, &loc),
                loc,
                msg: source.to_string(),
            });
        }
    };
    LitellmParams::try_from(map)
        .map_err(|source| BodyError::params(bytes, &[GUARDRAIL, LITELLM_PARAMS], &source))
}

impl BodyError {
    fn invalid_json() -> Self {
        Self {
            issue_type: "json_invalid",
            loc: body_location(&[]),
            msg: JSON_DECODE_ERROR.to_owned(),
            input: JsonData::Object(BTreeMap::new()),
        }
    }

    fn serde_at(
        bytes: &[u8],
        base: &[&str],
        source: &serde_path_to_error::Error<serde_json::Error>,
    ) -> Self {
        let message = source.inner().to_string();
        let mut loc = body_location(base);
        append_path(&mut loc, source.path());
        if let Some(field) = message
            .strip_prefix("missing field `")
            .and_then(|rest| rest.split_once('`'))
            .map(|(field, _position)| field)
        {
            loc.push(Location::Field(field.to_owned()));
            let input = value_at(bytes, parent_of(&loc));
            return Self {
                issue_type: "missing",
                loc,
                msg: FIELD_REQUIRED.to_owned(),
                input,
            };
        }
        let (issue_type, msg) = match source.inner().classify() {
            Category::Syntax | Category::Eof => ("json_invalid", JSON_DECODE_ERROR.to_owned()),
            Category::Io | Category::Data => ("value_error", message),
        };
        let input = value_at(bytes, &loc);
        Self {
            issue_type,
            loc,
            msg,
            input,
        }
    }

    fn params(bytes: &[u8], base: &[&str], source: &GuardrailParamsError) -> Self {
        let mut loc = body_location(base);
        let field = match source {
            GuardrailParamsError::InvalidField { field }
            | GuardrailParamsError::MissingField { field } => Some(field.as_str()),
            _ => None,
        };
        if let Some(field) = field {
            for part in field.split('.').filter(|part| !part.is_empty()) {
                loc.push(Location::Field(part.to_owned()));
            }
        }
        if matches!(source, GuardrailParamsError::MissingField { .. }) {
            return Self {
                issue_type: "missing",
                input: value_at(bytes, parent_of(&loc)),
                loc,
                msg: FIELD_REQUIRED.to_owned(),
            };
        }
        Self {
            issue_type: "value_error",
            input: value_at(bytes, &loc),
            loc,
            msg: source.to_string(),
        }
    }

    fn info(bytes: &[u8], source: &serde_json::Error) -> Self {
        let loc = body_location(&[GUARDRAIL, GUARDRAIL_INFO]);
        Self {
            issue_type: "value_error",
            input: value_at(bytes, &loc),
            loc,
            msg: source.to_string(),
        }
    }
}

fn body_location(base: &[&str]) -> Vec<Location> {
    let mut loc = vec![Location::Field(BODY.to_owned())];
    for part in base {
        loc.push(Location::Field((*part).to_owned()));
    }
    loc
}

fn append_path(loc: &mut Vec<Location>, path: &Path) {
    for segment in path {
        match segment {
            Segment::Seq { index } => loc.push(Location::Position(*index)),
            Segment::Map { key } => loc.push(Location::Field(key.clone())),
            Segment::Enum { variant } => loc.push(Location::Field(variant.clone())),
            Segment::Unknown => {}
        }
    }
}

fn parent_of(loc: &[Location]) -> &[Location] {
    match loc.split_last() {
        Some((_last, parent)) => parent,
        None => loc,
    }
}

// FastAPI reports the value at the rejected path, so the path is resolved against the raw body
fn value_at(bytes: &[u8], loc: &[Location]) -> JsonData {
    let Ok(root) = serde_json::from_slice::<JsonData>(bytes) else {
        return JsonData::Object(BTreeMap::new());
    };
    let mut current = &root;
    for part in loc.iter().skip(1) {
        let next = match (part, current) {
            (Location::Field(name), JsonData::Object(fields)) => fields.get(name),
            (Location::Position(index), JsonData::Array(items)) => items.get(*index),
            (Location::Field(_) | Location::Position(_), _) => None,
        };
        let Some(next) = next else { break };
        current = next;
    }
    current.clone()
}

impl IntoResponse for BodyError {
    fn into_response(self) -> Response {
        let body = ValidationBody { detail: [self] };
        (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
    }
}
