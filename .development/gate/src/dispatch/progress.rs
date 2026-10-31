use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::dispatch::EarlierReview;
use crate::error::GateError;
use crate::spec::agents::Rung;

const FILE: &str = "progress.json";

#[derive(Serialize, Deserialize, Debug, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum NextRun {
    #[default]
    Fresh,
    Fix,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReviewerPin {
    pub(crate) runner: String,
    pub(crate) model: String,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerSession {
    pub(crate) runner: String,
    pub(crate) model: String,
    pub(crate) id: String,
}

// the fields from next on default, so a progress file saved before they existed still loads
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
    #[serde(default)]
    pub(crate) next: NextRun,
    #[serde(default)]
    pub(crate) fix_turns: u32,
    #[serde(default)]
    pub(crate) granted: u32,
    #[serde(default)]
    pub(crate) reviewer: Option<ReviewerPin>,
    #[serde(default)]
    pub(crate) session: Option<WorkerSession>,
}

impl Progress {
    pub(crate) fn load(journal: &Path, notes: Vec<String>) -> Result<Self, GateError> {
        let path = journal.join(FILE);
        match fs::read_to_string(&path) {
            Ok(text) => {
                let mut saved: Self =
                    serde_json::from_str(&text).map_err(|source| GateError::Json {
                        what: path.display().to_string(),
                        source,
                    })?;
                let fresh: Vec<String> = notes
                    .into_iter()
                    .filter(|note| !saved.feedback.contains(note))
                    .collect();
                saved.feedback.extend(fresh);
                Ok(saved)
            }
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
                next: NextRun::Fresh,
                fix_turns: 0,
                granted: 0,
                reviewer: None,
                session: None,
            }),
            Err(source) => Err(GateError::Read { path, source }),
        }
    }

    pub(crate) const fn grant(&mut self, ladder: &[Rung], runs: u32) {
        // a used-up ladder goes back to its last rung with its attempts and earlier grants spent, so only the new runs happen
        if let Some(last) = ladder.last()
            && runs > 0
            && self.rung >= ladder.len()
        {
            self.rung = ladder.len().saturating_sub(1);
            self.rung_used = last.attempts.saturating_add(self.granted);
        }
        self.granted = self.granted.saturating_add(runs);
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
