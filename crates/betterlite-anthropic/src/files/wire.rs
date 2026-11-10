use betterlite_openai::files::records::{
    FileDeleted, FileKind, FileObject, FileStatus, FilesPurpose,
};
use chrono::NaiveDate;
use serde::Deserialize;

use super::error::FilesError;

// Python cuts the timestamp at the seconds before parsing, so fractions and offsets never reach it
const TIMESTAMP_LENGTH: usize = 19;
const LATEST_YEAR: i64 = 9999;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct AnthropicFile {
    id: Option<String>,
    filename: Option<String>,
    size_bytes: Option<i64>,
    bytes: Option<i64>,
    purpose: Option<FilesPurpose>,
    created_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FileListReply {
    data: Option<Vec<AnthropicFile>>,
}

#[derive(Debug, Deserialize)]
struct DeleteReply {
    id: Option<String>,
}

pub fn decode_anthropic_file(bytes: &[u8], now_epoch: i64) -> Result<FileObject, FilesError> {
    serde_json::from_slice(bytes)
        .map_err(|source| FilesError::Decode { source })
        .map(|file| project(file, now_epoch))
}

pub fn decode_anthropic_file_list(
    bytes: &[u8],
    now_epoch: i64,
) -> Result<Vec<FileObject>, FilesError> {
    serde_json::from_slice(bytes)
        .map_err(|source| FilesError::Decode { source })
        .map(|reply: FileListReply| {
            reply
                .data
                .unwrap_or_default()
                .into_iter()
                .map(|file| project(file, now_epoch))
                .collect()
        })
}

pub fn decode_anthropic_delete(bytes: &[u8]) -> Result<FileDeleted, FilesError> {
    serde_json::from_slice(bytes)
        .map_err(|source| FilesError::Decode { source })
        .map(|reply: DeleteReply| FileDeleted {
            id: reply.id.unwrap_or_default(),
            object: FileKind::File,
            deleted: true,
        })
}

fn project(file: AnthropicFile, now_epoch: i64) -> FileObject {
    let created_at = file
        .created_at
        .as_deref()
        .and_then(parse_created_at)
        .unwrap_or(now_epoch);
    FileObject {
        id: file.id.unwrap_or_default(),
        bytes: file.size_bytes.or(file.bytes).unwrap_or(0),
        created_at,
        filename: file.filename.unwrap_or_default(),
        object: FileKind::File,
        purpose: file.purpose.unwrap_or(FilesPurpose::Messages),
        status: Some(FileStatus::Uploaded),
        expires_at: None,
        status_details: None,
    }
}

// Python's time.strptime("%Y-%m-%dT%H:%M:%S") rejects a year past four digits or outside 1..=9999
fn parse_created_at(value: &str) -> Option<i64> {
    let replaced = value.replace('Z', "+00:00");
    let truncated: String = replaced.chars().take(TIMESTAMP_LENGTH).collect();
    let mut rest = truncated.as_str();
    let year = take_digits(&mut rest, 4, 4)?;
    rest = rest.strip_prefix('-')?;
    let month = take_digits(&mut rest, 1, 2)?;
    rest = rest.strip_prefix('-')?;
    let day = take_digits(&mut rest, 1, 2)?;
    rest = rest.strip_prefix(['T', 't'])?;
    let hour = take_digits(&mut rest, 1, 2)?;
    rest = rest.strip_prefix(':')?;
    let minute = take_digits(&mut rest, 1, 2)?;
    rest = rest.strip_prefix(':')?;
    let second = take_digits(&mut rest, 1, 2)?;
    if !rest.is_empty() || !(1..=LATEST_YEAR).contains(&year) {
        return None;
    }
    if !(0..=23).contains(&hour) || !(0..=59).contains(&minute) || !(0..=61).contains(&second) {
        return None;
    }
    let date = NaiveDate::from_ymd_opt(
        i32::try_from(year).ok()?,
        u32::try_from(month).ok()?,
        u32::try_from(day).ok()?,
    )?;
    let midnight = date.and_hms_opt(0, 0, 0)?.and_utc().timestamp();
    let seconds = hour
        .checked_mul(3600)?
        .checked_add(minute.checked_mul(60)?)?
        .checked_add(second)?;
    midnight.checked_add(seconds)
}

fn take_digits(rest: &mut &str, min: usize, max: usize) -> Option<i64> {
    let mut value: i64 = 0;
    let mut taken = 0;
    let mut consumed = 0;
    for (offset, character) in rest.char_indices() {
        if taken == max {
            break;
        }
        let Some(digit) = character.to_digit(10) else {
            break;
        };
        value = value.checked_mul(10)?.checked_add(i64::from(digit))?;
        taken = taken.checked_add(1)?;
        consumed = offset.checked_add(character.len_utf8())?;
    }
    if taken < min {
        return None;
    }
    *rest = rest.get(consumed..)?;
    Some(value)
}
