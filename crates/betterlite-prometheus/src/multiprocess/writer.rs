use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use crate::collector_factory::collector::MetricUpdate;
use crate::collector_registry::metric::gauge_mode::GaugeMode;
use crate::collector_registry::metric::{CollectorDescription, CollectorKind};

use super::MultiprocessError;
use super::check_directory;

mod file;
mod series;
mod tests;

use file::FileState;

// values.py names a gauge's file after its multiprocess mode and every other metric after its type
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum FileKind {
    Counter,
    Histogram,
    Gauge(GaugeMode),
}

impl FileKind {
    fn of(description: &CollectorDescription) -> Self {
        match description.kind {
            CollectorKind::Counter => Self::Counter,
            CollectorKind::Gauge => Self::Gauge(description.multiprocess_mode),
            CollectorKind::Histogram => Self::Histogram,
        }
    }

    fn file_name(self, pid: u32) -> String {
        match self {
            Self::Counter => format!("counter_{pid}.db"),
            Self::Histogram => format!("histogram_{pid}.db"),
            Self::Gauge(mode) => format!("gauge_{}_{pid}.db", mode.as_str()),
        }
    }
}

#[derive(Debug)]
pub struct MultiProcessWriter {
    pub(crate) directory: PathBuf,
    pid: u32,
    state: Mutex<WriterState>,
}

#[derive(Debug, Default)]
struct WriterState {
    files: BTreeMap<FileKind, FileState>,
}

// what one locked write does: an update, or the zero entries a new label-less series creates
pub(crate) enum SeriesWrite {
    Update(MetricUpdate),
    Create,
}

impl MultiProcessWriter {
    // the folder is checked when the writer is built, as the collector checks its own; a file opens on its first write
    pub fn new(directory: PathBuf, pid: u32) -> Result<Self, MultiprocessError> {
        check_directory(&directory)?;
        Ok(Self {
            directory,
            pid,
            state: Mutex::new(WriterState::default()),
        })
    }

    pub(crate) fn update(
        &self,
        description: &CollectorDescription,
        label_values: &[String],
        update: MetricUpdate,
    ) {
        if update == MetricUpdate::Remove {
            return;
        }
        self.write_series(description, label_values, SeriesWrite::Update(update));
    }

    // a failure goes to the log, never to the caller
    pub(crate) fn write_series(
        &self,
        description: &CollectorDescription,
        label_values: &[String],
        write: SeriesWrite,
    ) {
        if let Err(error) = self.write_locked(description, label_values, write) {
            tracing::error!(%error, "writing a metric to the multiprocess folder failed");
        }
    }

    fn write_locked(
        &self,
        description: &CollectorDescription,
        label_values: &[String],
        write: SeriesWrite,
    ) -> Result<(), MultiprocessError> {
        let keys = series::keys(description, label_values)?;
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let file = open_file(
            &mut state,
            &self.directory,
            FileKind::of(description),
            self.pid,
        )?;
        match write {
            SeriesWrite::Update(update) => series::apply(file, &keys, update),
            SeriesWrite::Create => {
                for key in keys.created() {
                    file.ensure(key)?;
                }
                Ok(())
            }
        }
    }
}

fn open_file<'state>(
    state: &'state mut WriterState,
    directory: &Path,
    kind: FileKind,
    pid: u32,
) -> Result<&'state mut FileState, MultiprocessError> {
    match state.files.entry(kind) {
        Entry::Occupied(entry) => Ok(entry.into_mut()),
        Entry::Vacant(entry) => Ok(entry.insert(FileState::open(directory, kind, pid)?)),
    }
}
