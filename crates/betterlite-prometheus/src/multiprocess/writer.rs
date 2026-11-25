use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use crate::collector_factory::collector::MetricUpdate;
use crate::collector_registry::metric::{CollectorDescription, CollectorKind, GaugeMode};

use super::MultiprocessError;
use super::check_directory;

mod file;
mod series;
mod tests;

use file::FileState;
use series::SeriesKeys;

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

// MultiProcessValue: every value of one process lives in the folder's files, and each inc and set writes through at once
#[derive(Debug)]
pub struct MultiProcessWriter {
    directory: PathBuf,
    pid: u32,
    state: Mutex<WriterState>,
}

#[derive(Debug, Default)]
struct WriterState {
    files: BTreeMap<FileKind, FileState>,
}

// what one locked write does: an update, or the zero entries a new label-less series creates
enum SeriesWrite {
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

    #[must_use]
    pub const fn directory(&self) -> &Path {
        &self.directory
    }

    // one update: every key the series owns becomes a zero entry first, then the file takes what the update changes
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

    // prometheus_client builds the one series of a label-less metric together with the metric, so it exists before any update
    pub(crate) fn create_series(
        &self,
        description: &CollectorDescription,
        label_values: &[String],
    ) {
        self.write_series(description, label_values, SeriesWrite::Create);
    }

    // both writes take the one lock and open the kind's file the same way; a failure goes to the log, never to the caller
    fn write_series(
        &self,
        description: &CollectorDescription,
        label_values: &[String],
        write: SeriesWrite,
    ) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let opened = open_series(
            &mut state,
            &self.directory,
            self.pid,
            description,
            label_values,
        );
        let (file, keys) = match opened {
            Ok(open) => open,
            Err(error) => return log_failure(&error),
        };
        match write {
            SeriesWrite::Update(update) => {
                if let Err(error) = series::apply(file, &keys, update) {
                    log_failure(&error);
                }
            }
            SeriesWrite::Create => {
                for key in keys.created() {
                    if let Err(error) = file.ensure(key) {
                        log_failure(&error);
                        return;
                    }
                }
            }
        }
    }
}

// the kind's file, open or opened now, together with the keys this series owns in it
fn open_series<'state>(
    state: &'state mut WriterState,
    directory: &Path,
    pid: u32,
    description: &CollectorDescription,
    label_values: &[String],
) -> Result<(&'state mut FileState, SeriesKeys), MultiprocessError> {
    let kind = FileKind::of(description);
    let keys = series::keys(description, label_values)?;
    let file = open_file(state, directory, kind, pid)?;
    Ok((file, keys))
}

// the writer's one lock over the open files: a kind's file opens on its first write and stays open
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

// a failed write must not fail the request that moved the metric, so the file path and the error go to the log
fn log_failure(error: &MultiprocessError) {
    tracing::error!(%error, "writing a metric to the multiprocess folder failed");
}
