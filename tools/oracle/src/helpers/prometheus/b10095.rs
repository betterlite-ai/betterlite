use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prometheus::multiprocess::collector::MultiProcessCollector;
use betterlite_prometheus::multiprocess::files::{DbEntry, encode_db_file};
use betterlite_prometheus::multiprocess::merge::merge;
use betterlite_prometheus::scrape_endpoint::{
    MetricFamily, MetricSample, ScrapeSource, parse_scrape_request,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Call {
    Merge,
    Scrape,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EntrySpec {
    metric: String,
    sample: String,
    #[serde(default)]
    labels: BTreeMap<String, String>,
    help: String,
    value: f64,
    #[serde(default)]
    timestamp: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileSpec {
    name: String,
    entries: Vec<EntrySpec>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    files: Vec<FileSpec>,
    call: Call,
    accept: String,
}

#[derive(Serialize)]
struct SampleOut {
    name: String,
    labels: BTreeMap<String, String>,
    value: f64,
}

#[derive(Serialize)]
struct FamilyOut {
    name: String,
    help: String,
    kind: String,
    samples: Vec<SampleOut>,
}

#[derive(Serialize)]
struct MergeAnswer {
    files: Vec<String>,
    families: Vec<FamilyOut>,
}

#[derive(Serialize)]
struct ScrapeAnswer {
    files: Vec<String>,
    blocks: Vec<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let directory = tempfile::TempDir::new().or_value_error()?;
        let mut paths = Vec::new();
        let mut files = Vec::new();
        for spec in &input.files {
            let bytes = encode_db_file(&entries_of(spec));
            let path = directory.path().join(&spec.name);
            fs::write(&path, &bytes).or_value_error()?;
            files.push(used_hex(&bytes));
            paths.push(path);
        }
        match input.call {
            Call::Merge => {
                let merged = merge(&paths).or_value_error()?;
                serde_json::to_value(MergeAnswer {
                    files,
                    families: families_out(&merged),
                })
                .or_value_error()
            }
            Call::Scrape => {
                let collector = MultiProcessCollector::new(directory.path()).or_value_error()?;
                let scrape = parse_scrape_request(&input.accept, "", Vec::new());
                let rendered = ScrapeSource::Multiprocess(collector)
                    .render(&scrape)
                    .or_value_error()?;
                let text = String::from_utf8(rendered).or_value_error()?;
                serde_json::to_value(ScrapeAnswer {
                    files,
                    blocks: blocks(&text),
                })
                .or_value_error()
            }
        }
    }
}

fn entries_of(spec: &FileSpec) -> Vec<DbEntry> {
    let mut entries = Vec::new();
    for entry in &spec.entries {
        entries.push(DbEntry {
            key: mmap_key(&entry.metric, &entry.sample, &entry.labels, &entry.help),
            value: entry.value,
            timestamp: entry.timestamp,
        });
    }
    entries
}

// json.dumps([metric_name, name, labels, help_text], sort_keys=True), the key prometheus_client's mmap_dict writes into the file
fn mmap_key(metric: &str, sample: &str, labels: &BTreeMap<String, String>, help: &str) -> String {
    let mut key = String::from("[");
    push_string(&mut key, metric);
    key.push_str(", ");
    push_string(&mut key, sample);
    key.push_str(", {");
    let mut first = true;
    for (name, value) in labels {
        if !first {
            key.push_str(", ");
        }
        push_string(&mut key, name);
        key.push_str(": ");
        push_string(&mut key, value);
        first = false;
    }
    key.push_str("}, ");
    push_string(&mut key, help);
    key.push(']');
    key
}

// python's json.dumps writes its strings with ensure_ascii, so the key bytes match the ones mmap_dict.py writes there
fn push_string(key: &mut String, value: &str) {
    key.push('"');
    for character in value.chars() {
        match character {
            '"' => key.push_str("\\\""),
            '\\' => key.push_str("\\\\"),
            '\n' => key.push_str("\\n"),
            '\r' => key.push_str("\\r"),
            '\t' => key.push_str("\\t"),
            '\u{8}' => key.push_str("\\b"),
            '\u{c}' => key.push_str("\\f"),
            escaped if escaped < ' ' || !escaped.is_ascii() => {
                let _written = write!(key, "\\u{:04x}", u32::from(escaped));
            }
            plain => key.push(plain),
        }
    }
    key.push('"');
}

// the bytes up to the used length, which is what mmap_dict.py's read_all_values_from_file reads
fn used_hex(bytes: &[u8]) -> String {
    let used = bytes
        .get(..4)
        .and_then(<[u8]>::first_chunk::<4>)
        .and_then(|length| usize::try_from(i32::from_le_bytes(*length)).ok())
        .unwrap_or_default();
    let mut hex = String::new();
    for byte in bytes.get(..used).unwrap_or_default() {
        let _written = write!(hex, "{byte:02x}");
    }
    hex
}

fn families_out(families: &[MetricFamily]) -> Vec<FamilyOut> {
    let mut sorted: Vec<&MetricFamily> = families.iter().collect();
    sorted.sort_by(|left, right| left.name.cmp(&right.name));
    sorted
        .iter()
        .map(|family| FamilyOut {
            name: family.name.clone(),
            help: family.help.clone(),
            kind: family.kind.as_str().to_owned(),
            samples: samples_out(&family.samples),
        })
        .collect()
}

// the key text sorts the samples the way Python's sort does, since folder order differs between the two folders
fn samples_out(samples: &[MetricSample]) -> Vec<SampleOut> {
    let mut rendered: Vec<SampleOut> = samples
        .iter()
        .map(|sample| SampleOut {
            name: sample.name.clone(),
            labels: sample.labels.iter().cloned().collect(),
            value: sample.value,
        })
        .collect();
    rendered.sort_by(|left, right| {
        let left_key = mmap_key(&left.name, "", &left.labels, "");
        let right_key = mmap_key(&right.name, "", &right.labels, "");
        left_key.cmp(&right_key)
    });
    rendered
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
