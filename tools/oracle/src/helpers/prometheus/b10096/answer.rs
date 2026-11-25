use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_prometheus::multiprocess::collector::MultiProcessCollector;
use betterlite_prometheus::multiprocess::files::decode_db_file;
use betterlite_prometheus::scrape_endpoint::{ScrapeSource, parse_scrape_request};
use serde::{Deserialize, Serialize};

// the [metric name, sample name, labels, help] array a key holds, which both writers parse back out of the file
#[derive(Deserialize, Serialize)]
struct KeyParts(String, String, BTreeMap<String, String>, String);

#[derive(Serialize)]
struct EntryOut {
    key: KeyParts,
    value: f64,
    timestamp: f64,
}

#[derive(Serialize)]
struct FileOut {
    name: String,
    length: usize,
    used: usize,
    entries: Vec<EntryOut>,
}

#[derive(Serialize)]
struct FilesAnswer {
    files: Vec<FileOut>,
}

#[derive(Serialize)]
struct ScrapeAnswer {
    files: Vec<String>,
    blocks: Vec<String>,
}

pub(super) fn files_answer(directory: &Path) -> Result<serde_json::Value, Box<HelperError>> {
    let mut names = metric_file_names(directory)?;
    names.sort();
    let mut files = Vec::with_capacity(names.len());
    for name in names {
        files.push(file_out(directory, &name)?);
    }
    serde_json::to_value(FilesAnswer { files }).or_value_error()
}

fn file_out(directory: &Path, name: &str) -> Result<FileOut, Box<HelperError>> {
    let bytes = fs::read(directory.join(name)).or_value_error()?;
    let mut entries = Vec::new();
    for entry in decode_db_file(&bytes).or_value_error()? {
        let key: KeyParts = serde_json::from_str(&entry.key).or_value_error()?;
        entries.push(EntryOut {
            key,
            value: entry.value,
            timestamp: entry.timestamp,
        });
    }
    Ok(FileOut {
        name: name.to_owned(),
        length: bytes.len(),
        used: used_length(&bytes),
        entries,
    })
}

pub(super) fn scrape_answer(
    directory: &Path,
    accept: &str,
) -> Result<serde_json::Value, Box<HelperError>> {
    let collector = MultiProcessCollector::new(directory).or_value_error()?;
    let scrape = parse_scrape_request(accept, "", Vec::new());
    let rendered = ScrapeSource::Multiprocess(collector)
        .render(&scrape)
        .or_value_error()?;
    let text = String::from_utf8(rendered).or_value_error()?;
    let mut names = metric_file_names(directory)?;
    names.sort();
    let mut files = Vec::with_capacity(names.len());
    for name in names {
        let bytes = fs::read(directory.join(&name)).or_value_error()?;
        files.push(used_hex(&bytes));
    }
    serde_json::to_value(ScrapeAnswer {
        files,
        blocks: blocks(&text),
    })
    .or_value_error()
}

fn metric_file_names(directory: &Path) -> Result<Vec<String>, Box<HelperError>> {
    let mut names = Vec::new();
    for listed in fs::read_dir(directory).or_value_error()? {
        let entry = listed.or_value_error()?;
        let path = entry.path();
        if path.extension() != Some(OsStr::new("db")) {
            continue;
        }
        let Some(name) = path.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        names.push(name.to_owned());
    }
    Ok(names)
}

// the used length in the file's first four bytes, which every reader trusts over the file size
fn used_length(bytes: &[u8]) -> usize {
    bytes
        .get(..4)
        .and_then(<[u8]>::first_chunk::<4>)
        .and_then(|length| usize::try_from(i32::from_le_bytes(*length)).ok())
        .unwrap_or_default()
}

// the bytes up to the used length, which is what mmap_dict.py's read_all_values_from_file reads
fn used_hex(bytes: &[u8]) -> String {
    let used = used_length(bytes);
    let mut hex = String::new();
    for byte in bytes.get(..used).unwrap_or_default() {
        let _written = write!(hex, "{byte:02x}");
    }
    hex
}

// the render split at each HELP line, the families sorted by name and the lines of each block sorted
fn blocks(text: &str) -> Vec<String> {
    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("# HELP ") {
            let name = rest.split(' ').next().unwrap_or_default().to_owned();
            sections.push((name, Vec::new()));
        }
        if line == "# EOF" {
            sections.push(("# EOF".to_owned(), Vec::new()));
        }
        if let Some(current) = sections.last_mut() {
            current.1.push(line.to_owned());
        }
    }
    let mut rendered: Vec<(String, String)> = Vec::new();
    for (name, mut lines) in sections {
        lines.sort();
        rendered.push((name, lines.join("\n")));
    }
    rendered.sort_by(|left, right| left.0.cmp(&right.0));
    rendered.into_iter().map(|(_name, block)| block).collect()
}
