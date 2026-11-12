use betterlite_openai::files::records::{
    FileDeleted, FileKind, FileObject, FileStatus, FilesPurpose,
};
use chrono::NaiveDate;
use serde::Deserialize;

use crate::api::file_contracts::config::{FileParams, FileResponse};

use super::error::ManusFilesError;

pub const INITIAL_FILE_RESPONSE_KEY: &str = "initial_file_response";

// Python cuts the timestamp at the seconds before parsing, so fractions and offsets never reach it.
const TIMESTAMP_LENGTH: usize = 19;
const DEFAULT_PURPOSE: FilesPurpose = FilesPurpose::Assistants;

#[derive(Debug, Deserialize)]
struct ManusFile {
    id: Option<String>,
    bytes: Option<i64>,
    created_at: Option<String>,
    filename: Option<String>,
    purpose: Option<FilesPurpose>,
    status_details: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ManusListItem {
    #[serde(flatten)]
    file: ManusFile,
    status: Option<FileStatus>,
}

#[derive(Debug, Deserialize)]
struct ManusFileList {
    data: Option<Vec<ManusListItem>>,
}

pub(super) fn create_file_object(
    raw_response: &FileResponse,
    litellm_params: &FileParams,
    now: i64,
) -> Result<FileObject, ManusFilesError> {
    match litellm_params
        .get(INITIAL_FILE_RESPONSE_KEY)
        .filter(|value| !value.is_null())
        .cloned()
    {
        Some(stored) => Ok(file_object(
            serde_json::from_value::<ManusFile>(stored).map_err(parse_failure)?,
            FileStatus::Uploaded,
            now,
        )),
        None => decode_create_file(&raw_response.body, now),
    }
}

pub fn decode_create_file(bytes: &[u8], now: i64) -> Result<FileObject, ManusFilesError> {
    Ok(file_object(
        serde_json::from_slice::<ManusFile>(bytes).map_err(parse_failure)?,
        FileStatus::Uploaded,
        now,
    ))
}

pub fn decode_file_list(bytes: &[u8], now: i64) -> Result<Vec<FileObject>, ManusFilesError> {
    let reply: ManusFileList =
        serde_json::from_slice(bytes).map_err(|source| ManusFilesError::ListResponse { source })?;
    Ok(reply
        .data
        .unwrap_or_default()
        .into_iter()
        .map(|item| file_object(item.file, item.status.unwrap_or(FileStatus::Uploaded), now))
        .collect())
}

pub fn decode_deleted_file(bytes: &[u8]) -> Result<FileDeleted, ManusFilesError> {
    serde_json::from_slice::<FileDeleted>(bytes)
        .map_err(|source| ManusFilesError::DeleteResponse { source })
}

// LiteLLM logs the failure at exception level before turning it into the ValueError.
fn parse_failure(source: serde_json::Error) -> ManusFilesError {
    tracing::error!("Error parsing Manus file response: {source}");
    ManusFilesError::ParseResponse { source }
}

fn file_object(file: ManusFile, status: FileStatus, now: i64) -> FileObject {
    let created_at = file
        .created_at
        .as_deref()
        .filter(|text| !text.is_empty())
        .and_then(parse_created_at)
        .unwrap_or(now);
    FileObject {
        id: file.id.unwrap_or_default(),
        bytes: file.bytes.unwrap_or(0),
        created_at,
        filename: file.filename.unwrap_or_default(),
        object: FileKind::File,
        purpose: file.purpose.unwrap_or(DEFAULT_PURPOSE),
        status: Some(status),
        expires_at: None,
        status_details: file.status_details,
    }
}

// Python parses %Y-%m-%dT%H:%M:%S after replacing Z, so the text keeps its own UTC seconds on a UTC host.
fn parse_created_at(value: &str) -> Option<i64> {
    let replaced = value.replace('Z', "+00:00");
    let cut: String = replaced.chars().take(TIMESTAMP_LENGTH).collect();
    let mut text = cut.as_str();
    let year = take_digits(&mut text, 4, 4)?;
    text = text.strip_prefix('-')?;
    let month = take_digits(&mut text, 1, 2)?;
    text = text.strip_prefix('-')?;
    let day = take_day(&mut text)?;
    text = text.strip_prefix(['T', 't'])?;
    let hour = take_digits(&mut text, 1, 2)?;
    text = text.strip_prefix(':')?;
    let minute = take_digits(&mut text, 1, 2)?;
    text = text.strip_prefix(':')?;
    let second = take_digits(&mut text, 1, 2)?;
    if !text.is_empty() || year == 0 {
        return None;
    }
    if !(0..=23).contains(&hour) || !(0..=59).contains(&minute) || !(0..=61).contains(&second) {
        return None;
    }
    // strptime rejects a day past the month's end and year 0, so LiteLLM falls back to now: https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/manus/files/transformation.py#L255-L268
    let date = NaiveDate::from_ymd_opt(
        i32::try_from(year).ok()?,
        u32::try_from(month).ok()?,
        u32::try_from(day).ok()?,
    )?;
    let elapsed = hour
        .checked_mul(3600)?
        .checked_add(minute.checked_mul(60)?)?
        .checked_add(second)?;
    date.and_hms_opt(0, 0, 0)?
        .and_utc()
        .timestamp()
        .checked_add(elapsed)
}

// strptime's %d also takes one space before a single-digit day.
fn take_day(text: &mut &str) -> Option<i64> {
    let Some(rest) = text.strip_prefix(' ') else {
        return take_digits(text, 1, 2);
    };
    *text = rest;
    take_digits(text, 1, 1).filter(|day| *day != 0)
}

fn take_digits(text: &mut &str, min: usize, max: usize) -> Option<i64> {
    let count = text.bytes().take_while(u8::is_ascii_digit).count().min(max);
    if count < min {
        return None;
    }
    let (digits, rest) = text.split_at(count);
    *text = rest;
    digits.parse().ok()
}
