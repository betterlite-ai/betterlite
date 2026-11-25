use std::collections::BTreeMap;
use std::fs;
use std::ops::Add;
use std::os::unix::fs::FileExt as _;
use std::path::{Path, PathBuf};

use crate::multiprocess::MultiprocessError;
use crate::multiprocess::files::{
    HEADER_BYTES, INITIAL_CAPACITY, TWO_DOUBLES_BYTES, USED_OFFSET, decode_db_values, decode_used,
    encode_entry, encode_used, encode_value,
};

use super::FileKind;

#[derive(Debug)]
pub(crate) struct FileState {
    path: PathBuf,
    file: fs::File,
    used: usize,
    capacity: usize,
    values: BTreeMap<String, StoredValue>,
}

#[derive(Debug)]
struct StoredValue {
    position: usize,
    value: f64,
}

impl StoredValue {
    const fn new(position: usize, value: f64) -> Self {
        Self { position, value }
    }
}

#[derive(Clone, Copy)]
pub(super) enum ValueChange {
    Add(f64),
    Replace(f64),
}

impl FileState {
    pub(super) fn open(
        directory: &Path,
        kind: FileKind,
        pid: u32,
    ) -> Result<Self, MultiprocessError> {
        let path = directory.join(kind.file_name(pid));
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|source| MultiprocessError::Write {
                path: path.clone(),
                source,
            })?;
        let length = file
            .metadata()
            .map_err(|source| MultiprocessError::Write {
                path: path.clone(),
                source,
            })?
            .len();
        let Ok(capacity) = usize::try_from(length) else {
            return Err(MultiprocessError::Corrupted);
        };
        if capacity > 0 {
            return rebuilt(path, file, capacity);
        }
        // a file that did not exist yet starts as one page whose zeroed header reads as no entries
        let page =
            u64::try_from(INITIAL_CAPACITY).map_err(|_error| MultiprocessError::Corrupted)?;
        file.set_len(page)
            .map_err(|source| MultiprocessError::Write {
                path: path.clone(),
                source,
            })?;
        rebuilt(path, file, INITIAL_CAPACITY)
    }

    pub(super) fn ensure(&mut self, key: &str) -> Result<(), MultiprocessError> {
        if self.values.contains_key(key) {
            return Ok(());
        }
        let entry = encode_entry(key, 0.0, 0.0);
        let end = self.used.saturating_add(entry.len());
        while end > self.capacity {
            self.capacity = self.capacity.saturating_mul(2);
            let grown =
                u64::try_from(self.capacity).map_err(|_error| MultiprocessError::Corrupted)?;
            self.file
                .set_len(grown)
                .map_err(|source| MultiprocessError::Write {
                    path: self.path.clone(),
                    source,
                })?;
        }
        write_at(&self.file, self.used, &entry, &self.path)?;
        self.used = end;
        // used goes in only after the entry bytes, so a reader never sees a used that covers half an entry
        let used_field = encode_used(self.used)?;
        write_at(&self.file, USED_OFFSET, &used_field, &self.path)?;
        let position = self.used.saturating_sub(TWO_DOUBLES_BYTES);
        let _inserted = self
            .values
            .insert(key.to_owned(), StoredValue::new(position, 0.0));
        Ok(())
    }

    pub(super) fn change(
        &mut self,
        key: &str,
        change: ValueChange,
    ) -> Result<(), MultiprocessError> {
        let Some(stored) = self.values.get_mut(key) else {
            return Err(MultiprocessError::MissingKey {
                key: key.to_owned(),
            });
        };
        stored.value = match change {
            ValueChange::Add(amount) => Add::add(stored.value, amount),
            ValueChange::Replace(value) => value,
        };
        let bytes = encode_value(stored.value, 0.0);
        write_at(&self.file, stored.position, &bytes, &self.path)
    }
}

// continue from the used length a file left, or start it at the header when that length is zero
fn rebuilt(path: PathBuf, file: fs::File, capacity: usize) -> Result<FileState, MultiprocessError> {
    let bytes = fs::read(&path).map_err(|source| MultiprocessError::Read {
        path: path.clone(),
        source,
    })?;
    let header = decode_used(&bytes)?;
    if header == 0 {
        // used goes in before any entry, so a file that never grew reads as holding none
        let used_field = encode_used(HEADER_BYTES)?;
        write_at(&file, USED_OFFSET, &used_field, &path)?;
        return Ok(FileState {
            path,
            file,
            used: HEADER_BYTES,
            capacity,
            values: BTreeMap::new(),
        });
    }
    Ok(FileState {
        path,
        file,
        used: header,
        capacity,
        values: stored_values(&bytes)?,
    })
}

fn stored_values(bytes: &[u8]) -> Result<BTreeMap<String, StoredValue>, MultiprocessError> {
    let mut values = BTreeMap::new();
    for (entry, position) in decode_db_values(bytes)? {
        let _replaced = values.insert(entry.key, StoredValue::new(position, entry.value));
    }
    Ok(values)
}

// write_all_at writes at an offset without mapping the file, so nothing here needs unsafe; std's calls take a u64 offset
fn write_at(
    file: &fs::File,
    position: usize,
    bytes: &[u8],
    path: &Path,
) -> Result<(), MultiprocessError> {
    let at = u64::try_from(position).map_err(|_error| MultiprocessError::Corrupted)?;
    file.write_all_at(bytes, at)
        .map_err(|source| MultiprocessError::Write {
            path: path.to_path_buf(),
            source,
        })
}
