use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use betterlite_models::decisions::Json;

const DEFAULT_MESSAGE_VALUE: &str = "default-message-value";

// A file handle cannot ride in a JSON request, so Rust callers pass it beside the document mapping
pub enum OcrFile {
    Path(PathBuf),
    Bytes(Vec<u8>),
    Named(String),
}

#[must_use]
pub fn ocr_document_summary(document: Option<&Json>, file: Option<&OcrFile>) -> String {
    let Some(Json::Object(fields)) = document else {
        return DEFAULT_MESSAGE_VALUE.to_owned();
    };
    let location = fields
        .get("document_url")
        .or_else(|| fields.get("image_url"));
    if let Some(Json::Text(text)) = location {
        if let Some((header, payload)) = text.split_once(',')
            && header.starts_with("data:")
        {
            return format!("{header} ({} chars)", payload.chars().count());
        }
        return text.clone();
    }
    let kind = match fields.get("mime_type") {
        Some(Json::Text(mime)) => format!("file ({mime})"),
        _ => String::from("file"),
    };
    match file {
        Some(OcrFile::Path(path)) => format!("{kind} {}", file_name(path)),
        Some(OcrFile::Bytes(bytes)) => format!("{kind} {} bytes", bytes.len()),
        Some(OcrFile::Named(name)) => format!("{kind} {}", file_name(Path::new(name))),
        None => kind,
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_owned()
}
