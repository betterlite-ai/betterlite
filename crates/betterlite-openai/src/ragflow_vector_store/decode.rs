use std::collections::BTreeMap;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::{Map, Number, Value};

use crate::api::vector_store_records::records::basic::VectorStoreStatus;
use crate::api::vector_store_records::records::vector::{
    VectorStoreCreateResponse, VectorStoreFileCounts,
};

use super::error::{CreateResponseError, RagflowError, ResponseHeaders};

const OBJECT: &str = "vector_store";
const MILLISECONDS_PER_SECOND: u32 = 1000;

pub fn decode_ragflow_create(
    status: u16,
    headers: &ResponseHeaders,
    body: &[u8],
) -> Result<VectorStoreCreateResponse, RagflowError> {
    let parsed: Value = serde_json::from_slice(body)
        .map_err(|source| unreadable(status, headers, CreateResponseError::Body { source }))?;
    let Value::Object(response) = parsed else {
        return Err(unreadable(
            status,
            headers,
            CreateResponseError::NotAnObject,
        ));
    };
    if !code_is_zero(&response) {
        let message = response
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Unknown error");
        return Err(RagflowError::ErrorResponse {
            status,
            headers: headers.clone(),
            message: message.to_owned(),
        });
    }
    let data = data_at(&response, status, headers)?;
    let id = data
        .and_then(|fields| fields.get("id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or(RagflowError::MissingDatasetId)?;
    let name = data
        .and_then(|fields| fields.get("name"))
        .and_then(Value::as_str);
    let created_at = created_at(
        data.and_then(|fields| fields.get("create_time")),
        status,
        headers,
    )?;
    Ok(create_response(id, name, created_at))
}

fn code_is_zero(response: &Map<String, Value>) -> bool {
    match response.get("code") {
        Some(Value::Number(code)) => parse_decimal(code).is_some_and(|parsed| parsed.is_zero()),
        _ => false,
    }
}

fn data_at<'response>(
    response: &'response Map<String, Value>,
    status: u16,
    headers: &ResponseHeaders,
) -> Result<Option<&'response Map<String, Value>>, RagflowError> {
    match response.get("data") {
        None => Ok(None),
        Some(Value::Object(data)) => Ok(Some(data)),
        Some(_) => Err(unreadable(status, headers, CreateResponseError::Data)),
    }
}

fn created_at(
    raw: Option<&Value>,
    status: u16,
    headers: &ResponseHeaders,
) -> Result<i64, RagflowError> {
    let Some(value) = raw.filter(|candidate| !candidate.is_null()) else {
        return Ok(0);
    };
    let Value::Number(number) = value else {
        return Err(unreadable(status, headers, CreateResponseError::CreateTime));
    };
    let milliseconds = parse_decimal(number)
        .ok_or_else(|| unreadable(status, headers, CreateResponseError::CreateTime))?;
    let seconds = milliseconds
        .checked_div(Decimal::from(MILLISECONDS_PER_SECOND))
        .ok_or_else(|| unreadable(status, headers, CreateResponseError::CreateTimeRange))?
        .trunc();
    seconds
        .to_i64()
        .ok_or_else(|| unreadable(status, headers, CreateResponseError::CreateTimeRange))
}

fn parse_decimal(number: &Number) -> Option<Decimal> {
    let lexeme = number.to_string();
    Decimal::from_str_exact(&lexeme)
        .or_else(|_| Decimal::from_scientific(&lexeme))
        .ok()
}

fn create_response(id: &str, name: Option<&str>, created_at: i64) -> VectorStoreCreateResponse {
    let mut extra = BTreeMap::new();
    if name.is_none() {
        extra.insert("name".to_owned(), Value::Null);
    }
    extra.insert("expires_after".to_owned(), Value::Null);
    extra.insert("expires_at".to_owned(), Value::Null);
    extra.insert("last_active_at".to_owned(), Value::Null);
    extra.insert("metadata".to_owned(), Value::Null);
    VectorStoreCreateResponse {
        id: Some(id.to_owned()),
        object: Some(OBJECT.to_owned()),
        created_at: Some(created_at),
        name: name.map(str::to_owned),
        bytes: Some(0),
        file_counts: Some(VectorStoreFileCounts {
            in_progress: Some(0),
            completed: Some(0),
            failed: Some(0),
            cancelled: Some(0),
            total: Some(0),
            extra: BTreeMap::new(),
        }),
        status: Some(VectorStoreStatus::Completed),
        expires_after: None,
        expires_at: None,
        last_active_at: None,
        metadata: None,
        extra,
    }
}

fn unreadable(status: u16, headers: &ResponseHeaders, source: CreateResponseError) -> RagflowError {
    RagflowError::UnreadableResponse {
        status,
        headers: headers.clone(),
        source,
    }
}
