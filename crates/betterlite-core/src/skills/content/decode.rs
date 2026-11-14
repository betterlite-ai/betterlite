use std::io::{self, Cursor, Read as _};
use std::str::Utf8Error;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_db::managed_resource_rows::skills::SkillTable;
use thiserror::Error;
use zip::ZipArchive;
use zip::result::ZipError;

pub(crate) const MAX_COMPRESSED_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_EXPANDED_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) const MAX_ARCHIVE_ENTRIES: usize = 4096;

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("the stored file content is not valid Base64")]
    StoredContent(#[source] base64::DecodeError),
    #[error("the archive is {size} bytes, past the {limit} byte limit")]
    ArchiveTooLarge { size: usize, limit: usize },
    #[error("the archive holds {count} entries, past the {limit} entry limit")]
    TooManyEntries { count: usize, limit: usize },
    #[error("the stored bytes are not a readable ZIP archive")]
    NotAnArchive(#[source] ZipError),
    #[error("entry {index} is missing from the archive")]
    MissingEntry { index: usize },
    #[error("entry {index} has a name that is not text")]
    NameNotText {
        index: usize,
        #[source]
        source: ZipError,
    },
    #[error("entry {name} cannot be opened")]
    Unreadable {
        name: String,
        #[source]
        source: ZipError,
    },
    #[error("entry {name} cannot be read")]
    Broken {
        name: String,
        #[source]
        source: io::Error,
    },
    #[error("entry {name} is not valid UTF-8")]
    NotText {
        name: String,
        #[source]
        source: Utf8Error,
    },
    #[error("the archive expands past the {limit} byte budget")]
    ExpansionPastLimit { limit: u64 },
}

pub struct SkillArchive {
    inner: ZipArchive<Cursor<Vec<u8>>>,
    expanded: u64,
}

pub fn decode_skill_zip(bytes: Vec<u8>) -> Result<SkillArchive, DecodeError> {
    if bytes.len() > MAX_COMPRESSED_BYTES {
        return Err(DecodeError::ArchiveTooLarge {
            size: bytes.len(),
            limit: MAX_COMPRESSED_BYTES,
        });
    }
    let inner = ZipArchive::new(Cursor::new(bytes)).map_err(DecodeError::NotAnArchive)?;
    if inner.len() > MAX_ARCHIVE_ENTRIES {
        return Err(DecodeError::TooManyEntries {
            count: inner.len(),
            limit: MAX_ARCHIVE_ENTRIES,
        });
    }
    Ok(SkillArchive { inner, expanded: 0 })
}

// B858 stores the skill row's file_content as Base64 text, the way Prisma hands it over
pub(crate) fn stored_archive_bytes(skill: &SkillTable) -> Result<Option<Vec<u8>>, DecodeError> {
    let Some(content) = skill.file_content.as_deref() else {
        return Ok(None);
    };
    if content.is_empty() {
        return Ok(None);
    }
    STANDARD
        .decode(content)
        .map(Some)
        .map_err(DecodeError::StoredContent)
}

pub(crate) fn open_skill_archive(skill: &SkillTable) -> Result<Option<SkillArchive>, DecodeError> {
    let Some(bytes) = stored_archive_bytes(skill)? else {
        return Ok(None);
    };
    decode_skill_zip(bytes).map(Some)
}

impl SkillArchive {
    pub fn entry_names(&self) -> Result<Vec<String>, DecodeError> {
        let count = self.inner.len();
        let mut names = Vec::with_capacity(count);
        for index in 0..count {
            let name = self
                .inner
                .name_for_index(index)
                .ok_or(DecodeError::MissingEntry { index })?
                .map_err(|source| DecodeError::NameNotText { index, source })?;
            names.push(name.into_owned());
        }
        Ok(names)
    }

    pub fn read_bytes(&mut self, index: usize, name: &str) -> Result<Vec<u8>, DecodeError> {
        let room = MAX_EXPANDED_BYTES.checked_sub(self.expanded).ok_or(
            DecodeError::ExpansionPastLimit {
                limit: MAX_EXPANDED_BYTES,
            },
        )?;
        let ceiling = room.checked_add(1).ok_or(DecodeError::ExpansionPastLimit {
            limit: MAX_EXPANDED_BYTES,
        })?;
        let mut file = self
            .inner
            .by_index(index)
            .map_err(|source| DecodeError::Unreadable {
                name: name.to_owned(),
                source,
            })?;
        let mut data = Vec::new();
        let mut limited = (&mut file).take(ceiling);
        let consumed = io::copy(&mut limited, &mut data).map_err(|source| DecodeError::Broken {
            name: name.to_owned(),
            source,
        })?;
        if consumed > room {
            return Err(DecodeError::ExpansionPastLimit {
                limit: MAX_EXPANDED_BYTES,
            });
        }
        self.expanded =
            self.expanded
                .checked_add(consumed)
                .ok_or(DecodeError::ExpansionPastLimit {
                    limit: MAX_EXPANDED_BYTES,
                })?;
        Ok(data)
    }

    pub fn read_text(&mut self, index: usize, name: &str) -> Result<String, DecodeError> {
        let data = self.read_bytes(index, name)?;
        String::from_utf8(data).map_err(|error| DecodeError::NotText {
            name: name.to_owned(),
            source: error.utf8_error(),
        })
    }
}
