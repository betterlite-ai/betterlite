use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::dispatch::EarlierReview;
use crate::error::GateError;

const FILE: &str = "progress.json";

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Progress {
    pub(crate) feedback: Vec<String>,
    pub(crate) number: u32,
    pub(crate) used: u32,
    pub(crate) rung: usize,
    pub(crate) rung_used: u32,
    pub(crate) limited: bool,
    pub(crate) review_rounds: u32,
    pub(crate) blocked_reports: u32,
    pub(crate) last_blocked: u32,
    pub(crate) earlier: Option<EarlierReview>,
    pub(crate) litellm: Vec<String>,
}

impl Progress {
    pub(crate) fn load(journal: &Path, notes: Vec<String>) -> Result<Self, GateError> {
        let path = journal.join(FILE);
        match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).map_err(|source| GateError::Json {
                what: path.display().to_string(),
                source,
            }),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Self {
                feedback: notes,
                number: 0,
                used: 0,
                rung: 0,
                rung_used: 0,
                limited: false,
                review_rounds: 0,
                blocked_reports: 0,
                last_blocked: 0,
                earlier: None,
                litellm: Vec::new(),
            }),
            Err(source) => Err(GateError::Read { path, source }),
        }
    }

    pub(crate) fn save(&self, journal: &Path) -> Result<(), GateError> {
        let text = serde_json::to_string_pretty(self).map_err(|source| GateError::Json {
            what: "the dispatch progress".to_owned(),
            source,
        })?;
        // a dispatch killed while writing must not leave half a file for the next one to read
        let part = journal.join("progress.json.part");
        fs::write(&part, text).map_err(|source| GateError::Write {
            path: part.clone(),
            source,
        })?;
        let path = journal.join(FILE);
        fs::rename(&part, &path).map_err(|source| GateError::Write { path, source })
    }
}

pub(crate) fn clear(journal: &Path) -> Result<(), GateError> {
    let path = journal.join(FILE);
    match fs::remove_file(&path) {
        Err(error) if error.kind() != ErrorKind::NotFound => Err(GateError::Write {
            path,
            source: error,
        }),
        _ => Ok(()),
    }
}
