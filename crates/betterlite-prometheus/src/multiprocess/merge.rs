use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use indexmap::map::Entry;
use serde::Deserialize;

use crate::scrape_endpoint::{MetricFamily, MetricKind};

use super::MultiprocessError;
use super::files::{DbEntry, decode_db_file};

mod accumulate;
mod tests;

type Families = IndexMap<String, FamilyBuilder>;

// one family as the files give it, before _accumulate_metrics combines its samples
struct FamilyBuilder {
    help: String,
    kind: MetricKind,
    mode: Option<GaugeMode>,
    samples: Vec<RawSample>,
}

// prometheus_client's gauge branches on the file name's mode; any other name takes the all branch
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GaugeMode {
    All,
    Min,
    Max,
    Sum,
    MostRecent,
}

impl GaugeMode {
    fn from_file_mode(mode: &str) -> Self {
        match mode {
            "min" | "livemin" => Self::Min,
            "max" | "livemax" => Self::Max,
            "sum" | "livesum" => Self::Sum,
            "mostrecent" | "livemostrecent" => Self::MostRecent,
            _other => Self::All,
        }
    }

    // min, max, sum and mostrecent combine a series without its pid label; all keeps each pid
    const fn combines_without_pid(self) -> bool {
        match self {
            Self::All => false,
            Self::Min | Self::Max | Self::Sum | Self::MostRecent => true,
        }
    }
}

struct RawSample {
    name: String,
    labels: Vec<(String, String)>,
    value: f64,
    timestamp: Option<f64>,
}

// the name and labels a sample accumulates under, in the order the files first give them
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct SampleKey {
    name: String,
    labels: Vec<(String, String)>,
}

// the [metric name, sample name, labels, help] array mmap_key writes
#[derive(Deserialize)]
struct KeyParts(String, String, BTreeMap<String, String>, String);

// MultiProcessCollector.merge(files, accumulate=True): _read_metrics then _accumulate_metrics
pub fn merge(files: &[PathBuf]) -> Result<Vec<MetricFamily>, MultiprocessError> {
    let mut families = Families::new();
    for path in files {
        read_file(&mut families, path)?;
    }
    accumulate(families)
}

fn read_file(families: &mut Families, path: &Path) -> Result<(), MultiprocessError> {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return Err(MultiprocessError::InvalidFileName {
            path: path.to_path_buf(),
        });
    };
    let parts: Vec<&str> = file_name.split('_').collect();
    let file_type = parts.first().copied().unwrap_or_default();
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error)
            if error.kind() == io::ErrorKind::NotFound
                && file_type == "gauge"
                && parts.get(1).is_some_and(|mode| mode.starts_with("live")) =>
        {
            return Ok(());
        }
        Err(source) => {
            return Err(MultiprocessError::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    for entry in decode_db_file(&bytes)? {
        add_entry(families, path, &parts, file_type, &entry)?;
    }
    Ok(())
}

fn add_entry(
    families: &mut Families,
    path: &Path,
    parts: &[&str],
    file_type: &str,
    entry: &DbEntry,
) -> Result<(), MultiprocessError> {
    let KeyParts(metric_name, sample_name, labels, help) = serde_json::from_str(&entry.key)
        .map_err(|source| MultiprocessError::InvalidKey { source })?;
    let family = family_or_create(families, &metric_name, &help, file_type)?;
    let mut label_pairs: Vec<(String, String)> = labels.into_iter().collect();
    let timestamp = if file_type == "gauge" {
        let Some(mode) = parts.get(1).copied() else {
            return Err(MultiprocessError::InvalidFileName {
                path: path.to_path_buf(),
            });
        };
        let Some(pid_part) = parts.get(2).copied() else {
            return Err(MultiprocessError::InvalidFileName {
                path: path.to_path_buf(),
            });
        };
        let pid = pid_part
            .get(..pid_part.len().saturating_sub(3))
            .unwrap_or_default();
        label_pairs.retain(|(label, _value)| label != "pid");
        label_pairs.push(("pid".to_owned(), pid.to_owned()));
        family.mode = Some(GaugeMode::from_file_mode(mode));
        Some(entry.timestamp)
    } else {
        None
    };
    family.samples.push(RawSample {
        name: sample_name,
        labels: label_pairs,
        value: entry.value,
        timestamp,
    });
    Ok(())
}

// Metric validates the name and the type only when it creates the family, and keeps the first file's type and help
fn family_or_create<'families>(
    families: &'families mut Families,
    metric_name: &str,
    help: &str,
    file_type: &str,
) -> Result<&'families mut FamilyBuilder, MultiprocessError> {
    match families.entry(metric_name.to_owned()) {
        Entry::Occupied(entry) => Ok(entry.into_mut()),
        Entry::Vacant(entry) => {
            if !is_valid_metric_name(metric_name) {
                return Err(MultiprocessError::InvalidName {
                    name: metric_name.to_owned(),
                });
            }
            let Some(kind) = MetricKind::from_type_name(file_type) else {
                return Err(MultiprocessError::InvalidType {
                    kind: file_type.to_owned(),
                });
            };
            Ok(entry.insert(FamilyBuilder {
                help: help.to_owned(),
                kind,
                mode: None,
                samples: Vec::new(),
            }))
        }
    }
}

// metrics_core.METRIC_NAME_RE: a letter, an underscore or a colon, then those and digits
fn is_valid_metric_name(name: &str) -> bool {
    match name.as_bytes().split_first() {
        Some((first, rest)) => {
            matches!(*first, b'a'..=b'z' | b'A'..=b'Z' | b'_' | b':')
                && rest.iter().all(|byte| {
                    matches!(
                        *byte,
                        b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b':'
                    )
                })
        }
        None => false,
    }
}

// the families in first-seen order, each with its accumulated samples
fn accumulate(families: Families) -> Result<Vec<MetricFamily>, MultiprocessError> {
    let mut accumulated = Vec::with_capacity(families.len());
    for (name, family) in families {
        let samples = accumulate::samples(&name, &family)?;
        accumulated.push(MetricFamily {
            name,
            help: family.help,
            kind: family.kind,
            samples,
        });
    }
    Ok(accumulated)
}
