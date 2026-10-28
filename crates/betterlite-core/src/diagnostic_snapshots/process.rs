use serde::Serialize;
use std::fs;
use std::io;
use std::ops::{Div as _, Mul as _};
use std::str;
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryStatus {
    Critical,
    Warning,
    Healthy,
}

#[must_use]
pub fn health_status(percent: f64) -> MemoryStatus {
    if percent > 80.0 {
        MemoryStatus::Critical
    } else if percent > 60.0 {
        MemoryStatus::Warning
    } else {
        MemoryStatus::Healthy
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ProcessMemory {
    pub resident_bytes: u64,
    pub virtual_bytes: u64,
    pub percent: f64,
}

#[derive(Debug, Error)]
pub enum ProcessMemoryError {
    #[error("unsupported runtime")]
    Unsupported,
    #[error("invalid /proc memory data: {0}")]
    Invalid(String),
    #[error("could not read process memory: {0}")]
    Io(#[source] io::Error),
}

#[derive(Debug, Serialize)]
pub struct ProcessMemorySnapshot {
    pub memory: Option<ProcessMemory>,
    pub status: MemoryStatus,
    pub error: Option<String>,
}

pub fn parse_proc_memory(
    statm: &[u8],
    meminfo: &[u8],
    page_size: u64,
) -> Result<ProcessMemory, ProcessMemoryError> {
    let mut statm_fields = statm
        .split(u8::is_ascii_whitespace)
        .filter(|field| !field.is_empty());
    let virtual_pages = statm_fields
        .next()
        .ok_or_else(|| ProcessMemoryError::Invalid("missing virtual pages".to_owned()))?;
    let resident_pages = statm_fields
        .next()
        .ok_or_else(|| ProcessMemoryError::Invalid("missing resident pages".to_owned()))?;
    let virtual_page_count = parse_number(virtual_pages, "virtual pages")?;
    let resident_page_count = parse_number(resident_pages, "resident pages")?;
    let total_kilobytes = meminfo
        .split(|byte| *byte == b'\n')
        .find_map(|line| {
            line.strip_prefix(b"MemTotal:").and_then(|value| {
                value
                    .split(u8::is_ascii_whitespace)
                    .find(|field| !field.is_empty())
                    .and_then(|field| parse_number(field, "MemTotal").ok())
            })
        })
        .ok_or_else(|| ProcessMemoryError::Invalid("missing MemTotal".to_owned()))?;
    let virtual_bytes = virtual_page_count
        .checked_mul(page_size)
        .ok_or_else(|| ProcessMemoryError::Invalid("virtual memory overflow".to_owned()))?;
    let resident_bytes = resident_page_count
        .checked_mul(page_size)
        .ok_or_else(|| ProcessMemoryError::Invalid("resident memory overflow".to_owned()))?;
    let total_bytes = total_kilobytes
        .checked_mul(1024)
        .ok_or_else(|| ProcessMemoryError::Invalid("system memory overflow".to_owned()))?;
    let resident = decimal_value(resident_bytes, "resident memory")?;
    let total = decimal_value(total_bytes, "system memory")?;
    let percent = resident.mul(100.0_f64).div(total);
    Ok(ProcessMemory {
        resident_bytes,
        virtual_bytes,
        percent,
    })
}

fn decimal_value(value: u64, name: &str) -> Result<f64, ProcessMemoryError> {
    match value.to_string().parse::<f64>() {
        Ok(decimal) => Ok(decimal),
        Err(error) => Err(ProcessMemoryError::Invalid(format!(
            "invalid {name}: {error}"
        ))),
    }
}

fn parse_number(value: &[u8], name: &str) -> Result<u64, ProcessMemoryError> {
    match str::from_utf8(value) {
        Ok(text) => match text.parse::<u64>() {
            Ok(number) => Ok(number),
            Err(error) => Err(ProcessMemoryError::Invalid(format!(
                "invalid {name}: {error}"
            ))),
        },
        Err(error) => Err(ProcessMemoryError::Invalid(format!(
            "invalid {name}: {error}"
        ))),
    }
}

#[must_use]
pub fn process_memory_snapshot(
    result: Result<ProcessMemory, ProcessMemoryError>,
) -> ProcessMemorySnapshot {
    match result {
        Ok(memory) => ProcessMemorySnapshot {
            status: health_status(memory.percent),
            memory: Some(memory),
            error: None,
        },
        Err(error) => ProcessMemorySnapshot {
            status: MemoryStatus::Healthy,
            memory: None,
            error: Some(error.to_string()),
        },
    }
}

#[must_use]
pub fn snapshot_process_memory() -> ProcessMemorySnapshot {
    let result = read_process_memory();
    process_memory_snapshot(result)
}

#[cfg(target_os = "linux")]
pub(super) fn read_process_memory() -> Result<ProcessMemory, ProcessMemoryError> {
    let statm = fs::read("/proc/self/statm").map_err(ProcessMemoryError::Io)?;
    let meminfo = fs::read("/proc/meminfo").map_err(ProcessMemoryError::Io)?;
    parse_proc_memory(&statm, &meminfo, 4096)
}

#[cfg(not(target_os = "linux"))]
pub(super) fn read_process_memory() -> Result<ProcessMemory, ProcessMemoryError> {
    let error = ProcessMemoryError::Unsupported;
    Err(error)
}
