use indexmap::IndexMap;
use thiserror::Error;

use super::values::{FormFields, FormValue, UploadFile};

// The (filename, content, content_type) tuple httpx and litellm's HTTP handlers take for one upload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileTuple {
    pub filename: String,
    pub content: Vec<u8>,
    pub content_type: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileDataValue {
    Unchanged(FormValue),
    Files(Vec<FileTuple>),
}

pub type FileData = IndexMap<String, FileDataValue>;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ConvertError {
    #[error(
        "the upload list of form field {name} holds a value without a read method at position {index}"
    )]
    MissingRead { name: String, index: usize },
}

// convert_upload_files_to_file_data: a list counts as uploads only when its first entry has read, so a mixed list is decided by that first entry alone.
pub fn convert_upload_files_to_file_data(fields: &FormFields) -> Result<FileData, ConvertError> {
    let mut data = FileData::new();
    for (name, value) in fields {
        let converted = match value {
            FormValue::List(items) => convert_list(name, items)?,
            FormValue::Upload(upload) => FileDataValue::Files(vec![file_tuple(upload)]),
            _other => FileDataValue::Unchanged(value.clone()),
        };
        data.insert(name.clone(), converted);
    }
    Ok(data)
}

fn convert_list(name: &str, items: &[FormValue]) -> Result<FileDataValue, ConvertError> {
    if !matches!(items.first(), Some(FormValue::Upload(_))) {
        return Ok(FileDataValue::Unchanged(FormValue::List(items.to_vec())));
    }
    let mut files = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let FormValue::Upload(upload) = item else {
            return Err(ConvertError::MissingRead {
                name: name.to_owned(),
                index,
            });
        };
        files.push(file_tuple(upload));
    }
    Ok(FileDataValue::Files(files))
}

fn file_tuple(upload: &UploadFile) -> FileTuple {
    FileTuple {
        filename: upload.filename.clone(),
        content: upload.content.clone(),
        content_type: upload.content_type.clone(),
    }
}
