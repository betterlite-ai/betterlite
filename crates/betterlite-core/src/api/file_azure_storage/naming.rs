use std::time::Duration;

use betterlite_logging::azure_storage_logging::filename::IdSource;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use crate::api::file_storage_contracts::storage::FileNamingStrategy;
use crate::api::upload_filename::safe_filename;

// Python's quote() leaves ALPHA, DIGIT and _.-~ alone, and every other byte becomes %XX
const FILENAME_SAFE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');
// the SDK quotes a blob path with quote(path), whose default safe set keeps / a separator
const PATH_SAFE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~')
    .remove(b'/');

// LiteLLM compares the raw strategy string and falls back to uuid for anything else
#[must_use]
pub fn naming_strategy(value: &str) -> FileNamingStrategy {
    match value {
        "original_filename" => FileNamingStrategy::OriginalFilename,
        "timestamp" => FileNamingStrategy::Timestamp,
        _ => FileNamingStrategy::Uuid,
    }
}

#[must_use]
pub fn generated_file_name(
    original_filename: &str,
    strategy: FileNamingStrategy,
    now: Duration,
    ids: &dyn IdSource,
) -> String {
    match strategy {
        FileNamingStrategy::OriginalFilename => quoted_basename(original_filename),
        FileNamingStrategy::Timestamp => {
            with_extension(&epoch_milliseconds(now), &safe_extension(original_filename))
        }
        FileNamingStrategy::Uuid => {
            with_extension(&ids.next_id(), &safe_extension(original_filename))
        }
    }
}

pub(super) fn quoted_path(path: &str) -> String {
    utf8_percent_encode(path, PATH_SAFE).to_string()
}

fn quoted_basename(original_filename: &str) -> String {
    utf8_percent_encode(&safe_basename(original_filename), FILENAME_SAFE).to_string()
}

// LiteLLM turns every rejected basename into the literal file, and then finds no extension in it
fn safe_basename(original_filename: &str) -> String {
    match safe_filename(original_filename) {
        Ok(name) => name,
        Err(_error) => "file".to_owned(),
    }
}

// PurePath(name).suffix.lstrip("."): the last dot wins, but not when it opens or ends the name
fn safe_extension(original_filename: &str) -> String {
    let name = safe_basename(original_filename);
    let Some(index) = name.rfind('.') else {
        return String::new();
    };
    if index == 0 || index + 1 == name.len() {
        return String::new();
    }
    name.get(index + 1..).unwrap_or_default().to_owned()
}

// int(time.time() * 1000): one float multiply, then truncation toward zero
fn epoch_milliseconds(now: Duration) -> String {
    let milliseconds = now.as_secs_f64().mul_add(1000.0, 0.0).trunc();
    format!("{milliseconds:.0}")
}

fn with_extension(stem: &str, extension: &str) -> String {
    if extension.is_empty() {
        return stem.to_owned();
    }
    format!("{stem}.{extension}")
}
