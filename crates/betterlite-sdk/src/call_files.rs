use std::collections::BTreeSet;
use std::mem::take;

use betterlite_models::decisions::Json;

use crate::call_setup::RequestMap;

mod tests;

const FILE_KEY: &str = "$file";
const BLOB_KEY: &str = "blob";
const NAME_KEY: &str = "name";
const CONTENT_TYPE_KEY: &str = "content_type";
const DEFAULT_NAME: &str = "file";

// One file a call sends beside its JSON text: the name and type a provider request needs, and the bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallFile {
    pub name: String,
    pub content_type: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum CallFilesError {
    #[error("the {key} file has no blob index")]
    MissingBlob { key: String },
    #[error("the {key} file has a blob index that is not a number")]
    InvalidIndex { key: String },
    #[error("the {key} file marker is not an object")]
    InvalidMarker { key: String },
    #[error("the {key} file names blob {index}, which the call did not send")]
    UnknownBlob { key: String, index: u64 },
    #[error("the {key} file uses blob {index} a second time")]
    DuplicateBlob { key: String, index: u64 },
}

// The payload's $file markers become CallFiles in key order, and each blob is moved out rather than copied.
pub fn files_from_payload(
    payload: &RequestMap,
    blobs: &mut [Vec<u8>],
) -> Result<Vec<(String, CallFile)>, CallFilesError> {
    let mut files = Vec::new();
    let mut used = BTreeSet::new();
    for (key, value) in payload {
        collect_files(key, value, blobs, &mut used, &mut files)?;
    }
    Ok(files)
}

fn collect_files(
    key: &str,
    value: &Json,
    blobs: &mut [Vec<u8>],
    used: &mut BTreeSet<usize>,
    files: &mut Vec<(String, CallFile)>,
) -> Result<(), CallFilesError> {
    match value {
        Json::Object(fields) => {
            if let Some(marker) = fields.get(FILE_KEY) {
                files.push((key.to_owned(), call_file(key, marker, blobs, used)?));
            } else {
                for (field, nested) in fields {
                    collect_files(field, nested, blobs, used, files)?;
                }
            }
        }
        Json::List(items) => {
            for item in items {
                collect_files(key, item, blobs, used, files)?;
            }
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {}
    }
    Ok(())
}

fn call_file(
    key: &str,
    marker: &Json,
    blobs: &mut [Vec<u8>],
    used: &mut BTreeSet<usize>,
) -> Result<CallFile, CallFilesError> {
    let Json::Object(fields) = marker else {
        return Err(CallFilesError::InvalidMarker {
            key: key.to_owned(),
        });
    };
    let index = match fields.get(BLOB_KEY) {
        None | Some(Json::Null) => {
            return Err(CallFilesError::MissingBlob {
                key: key.to_owned(),
            });
        }
        Some(Json::Number(number)) => {
            number
                .as_u64()
                .ok_or_else(|| CallFilesError::InvalidIndex {
                    key: key.to_owned(),
                })?
        }
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            return Err(CallFilesError::InvalidIndex {
                key: key.to_owned(),
            });
        }
    };
    let Ok(position) = usize::try_from(index) else {
        return Err(CallFilesError::UnknownBlob {
            key: key.to_owned(),
            index,
        });
    };
    let Some(blob) = blobs.get_mut(position) else {
        return Err(CallFilesError::UnknownBlob {
            key: key.to_owned(),
            index,
        });
    };
    let name = marker_text(key, fields.get(NAME_KEY))?.unwrap_or_else(|| DEFAULT_NAME.to_owned());
    let content_type = marker_text(key, fields.get(CONTENT_TYPE_KEY))?;
    if !used.insert(position) {
        return Err(CallFilesError::DuplicateBlob {
            key: key.to_owned(),
            index,
        });
    }
    Ok(CallFile {
        name,
        content_type,
        bytes: take(blob),
    })
}

fn marker_text(key: &str, value: Option<&Json>) -> Result<Option<String>, CallFilesError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(CallFilesError::InvalidMarker {
                key: key.to_owned(),
            })
        }
    }
}
