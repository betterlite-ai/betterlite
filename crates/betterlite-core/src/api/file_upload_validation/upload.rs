use crate::api::upload_filename::safe_filename;
use super::error::FileUploadValidationError;
use super::failure::{FILE_PARAM, FILE_TOO_LARGE, FailureFields, INVALID_REQUEST};
use super::source::FileBody;
use super::{exceeds, megabytes};
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadValidationFailure {
    TooLarge { size_bytes: u128, limit_mb: i64 },
    ExtensionNotAllowed { extension: String },
    BlockedExtension { extension: String },
    UnsafeFilename { filename: String },
}
impl UploadValidationFailure {
    pub fn fields(&self) -> FailureFields {
        let (status, param) = match self {
            Self::TooLarge { .. } => (FILE_TOO_LARGE, FILE_PARAM),
            Self::ExtensionNotAllowed { .. }
            | Self::BlockedExtension { .. }
            | Self::UnsafeFilename { .. } => (INVALID_REQUEST, FILE_PARAM),
        };
        FailureFields {
            message: self.message(),
            status,
            param: param.to_owned(),
        }
    }
    fn message(&self) -> String {
        match self {
            Self::TooLarge {
                size_bytes,
                limit_mb,
            } => format!(
                "Uploaded file exceeds the configured max_file_size_mb of {limit_mb} MB (read stopped at {} MB). The file was not forwarded to the provider.",
                megabytes(*size_bytes)
            ),
            Self::ExtensionNotAllowed { extension } => {
                if extension.is_empty() {
                    "A file without an extension is not in this proxy's allowed_file_extensions setting. The file was not forwarded to the provider.".to_owned()
                } else {
                    format!(
                        "File extension '{extension}' is not in this proxy's allowed_file_extensions setting. The file was not forwarded to the provider."
                    )
                }
            }
            Self::BlockedExtension { extension } => format!(
                "File extension '{extension}' is blocked by this proxy's blocked_file_extensions setting. The file was not forwarded to the provider."
            ),
            Self::UnsafeFilename { filename } => format!(
                "Filename '{filename}' is not allowed: directory traversal sequences are not permitted in uploaded file names. The file was not forwarded to the provider."
            ),
        }
    }
}
pub fn check_upload_file_size(
    body: &mut FileBody<'_>,
    max_file_size_mb: Option<i64>,
) -> Result<Option<UploadValidationFailure>, FileUploadValidationError> {
    let Some(limit_mb) = max_file_size_mb else {
        return Ok(None);
    };
    let size_bytes = body.size()?;
    if exceeds(size_bytes, limit_mb) {
        return Ok(Some(UploadValidationFailure::TooLarge {
            size_bytes,
            limit_mb,
        }));
    }
    Ok(None)
}
pub fn check_allowed_extension(
    filename: Option<&str>,
    allowed_extensions: Option<&[String]>,
) -> Option<UploadValidationFailure> {
    let allowed_extensions = allowed_extensions?;
    let extension = normalized_extension(filename);
    let permitted = allowed_extensions
        .iter()
        .any(|item| item.to_lowercase() == extension);
    if !extension.is_empty() && permitted {
        return None;
    }
    Some(UploadValidationFailure::ExtensionNotAllowed { extension })
}
pub fn check_blocked_extension(
    filename: Option<&str>,
    blocked_extensions: Option<&[String]>,
) -> Option<UploadValidationFailure> {
    let blocked_extensions = blocked_extensions?;
    if blocked_extensions.is_empty() {
        return None;
    }
    let extension = normalized_extension(filename);
    let blocked = !extension.is_empty()
        && blocked_extensions
            .iter()
            .any(|item| item.to_lowercase() == extension);
    if blocked {
        return Some(UploadValidationFailure::BlockedExtension { extension });
    }
    None
}
pub fn check_unsafe_filename(filename: Option<&str>) -> Option<UploadValidationFailure> {
    let Some(filename) = filename else {
        return None;
    };
    if filename.is_empty() {
        return None;
    }
    let traversal = filename
        .replace('\\', "/")
        .split('/')
        .any(|part| part == "..");
    if filename.contains('\0') || traversal {
        return Some(UploadValidationFailure::UnsafeFilename {
            filename: filename.to_owned(),
        });
    }
    None
}
fn normalized_extension(filename: Option<&str>) -> String {
    let Some(filename) = filename.filter(|name| !name.is_empty()) else {
        return String::new();
    };
    let Ok(base) = safe_filename(filename) else {
        return String::new();
    };
    let Some(index) = base.rfind('.') else {
        return String::new();
    };
    let Some(suffix) = base.get(index..) else {
        return String::new();
    };
    if index == 0 || suffix.len() <= 1 {
        return String::new();
    }
    suffix.to_lowercase()
}
