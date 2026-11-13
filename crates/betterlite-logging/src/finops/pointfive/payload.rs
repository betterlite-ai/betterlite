use std::io;
use std::io::Write as _;
use std::mem;

use flate2::Compression;
use flate2::write::GzEncoder;
use serde::Serialize;
use thiserror::Error;
use tokio::task::JoinError;
use tokio::task::JoinSet;

use crate::private_json::error::PrivateJsonError;
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};

// NDJSON counts a line as its UTF-8 bytes plus the one-byte separator that follows it
const NEWLINE_BYTES: usize = 1;

#[derive(Debug, Error)]
pub enum PayloadError {
    #[error("a record could not be serialized")]
    Serialize(#[source] PrivateJsonError),
    #[error("the batch could not be gzipped")]
    Compress(#[from] io::Error),
    #[error("the compression worker failed")]
    Worker(#[source] JoinError),
    #[error("the compression worker stopped before it finished")]
    WorkerStopped,
}

pub fn serialize_records<Record>(records: &[Record]) -> Result<Vec<String>, PayloadError>
where
    Record: Serialize,
{
    records
        .iter()
        .map(|record| {
            safe_dumps(record, DEFAULT_MAX_RECURSE_DEPTH, None).map_err(PayloadError::Serialize)
        })
        .collect()
}

// An oversized line keeps its own object; holding it back would stall every record behind it
#[must_use]
pub fn chunk_lines(lines: &[String], max_bytes: usize) -> Vec<Vec<&str>> {
    let mut chunks: Vec<Vec<&str>> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut used = 0_usize;
    for line in lines {
        let size = line.len().saturating_add(NEWLINE_BYTES);
        let overflow = used != 0 && used.checked_add(size).is_none_or(|total| total > max_bytes);
        if overflow {
            chunks.push(mem::take(&mut current));
            used = 0;
        }
        current.push(line.as_str());
        used = used.saturating_add(size);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

pub async fn encode_lines(lines: &[&str]) -> Result<Vec<u8>, PayloadError> {
    let document = lines.join("\n").into_bytes();
    let mut tasks = JoinSet::new();
    tasks.spawn_blocking(move || gzip(&document));
    let joined = tasks.join_next().await.ok_or(PayloadError::WorkerStopped)?;
    let compressed = joined.map_err(PayloadError::Worker)?;
    compressed.map_err(PayloadError::Compress)
}

fn gzip(document: &[u8]) -> Result<Vec<u8>, io::Error> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(document)?;
    encoder.finish()
}
