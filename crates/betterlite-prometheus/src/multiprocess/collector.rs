use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::scrape_endpoint::MetricFamily;

use super::MultiprocessError;
use super::check_directory;
use super::merge::merge;

mod tests;

// Gauge._MULTIPROC_MODES filtered by live: the modes whose file no longer means anything once the process is gone
const LIVE_GAUGE_MODES: [&str; 5] = ["liveall", "livemin", "livemax", "livesum", "livemostrecent"];

#[derive(Debug)]
pub struct MultiProcessCollector {
    path: PathBuf,
}

impl MultiProcessCollector {
    // LiteLLM's mount: the folder is checked when the app is built and read again at every scrape
    pub fn new(path: &Path) -> Result<Self, MultiprocessError> {
        check_directory(path)?;
        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    // MultiProcessCollector.collect: glob's *.db, so no dot file and nothing else, in read_dir order
    pub fn collect(&self) -> Result<Vec<MetricFamily>, MultiprocessError> {
        let entries = fs::read_dir(&self.path).map_err(|source| MultiprocessError::List {
            path: self.path.clone(),
            source,
        })?;
        let mut files = Vec::new();
        for listed in entries {
            let entry = listed.map_err(|source| MultiprocessError::List {
                path: self.path.clone(),
                source,
            })?;
            let path = entry.path();
            let Some(text) = path.file_name().and_then(OsStr::to_str) else {
                continue;
            };
            if text.starts_with('.') || path.extension() != Some(OsStr::new("db")) {
                continue;
            }
            files.push(path);
        }
        merge(&files)
    }
}

// mark_process_dead: every live gauge file of one dead pid, and a failure to remove one is an error
pub fn mark_process_dead(pid: u32, path: &Path) -> Result<(), MultiprocessError> {
    for mode in LIVE_GAUGE_MODES {
        let file = path.join(format!("gauge_{mode}_{pid}.db"));
        match fs::remove_file(&file) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(MultiprocessError::Remove { path: file, source }),
        }
    }
    Ok(())
}
