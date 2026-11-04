pub const MAX_UPLOAD_SIZE_BYTES: usize = 512 * 1024 * 1024;
pub const EICAR_TEST_SIGNATURE: &[u8] =
    b"X5O!P%@AP[4\\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectedFormat {
    Pdf,
    Text,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisallowedKind {
    Archive,
    Executable,
    UnknownBinary,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectionReason {
    EmptyFile,
    FileTooLarge,
    ArchiveNotAllowed,
    ExecutableNotAllowed,
    UnsupportedFormat,
    MalwareDetected,
    MalwareScanError,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanVerdict {
    Clean,
    Infected,
    Error,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanResult {
    pub verdict: ScanVerdict,
    pub signature: Option<String>,
}
pub trait MalwareScanner {
    fn scan(&self, content: &[u8]) -> ScanResult;
}
#[derive(Debug, Clone, Copy, Default)]
pub struct EicarTestMalwareScanner;
impl MalwareScanner for EicarTestMalwareScanner {
    fn scan(&self, content: &[u8]) -> ScanResult {
        let infected = content
            .windows(EICAR_TEST_SIGNATURE.len())
            .any(|window| window == EICAR_TEST_SIGNATURE);
        ScanResult {
            verdict: if infected {
                ScanVerdict::Infected
            } else {
                ScanVerdict::Clean
            },
            signature: infected.then(|| "EICAR-STANDARD-ANTIVIRUS-TEST-FILE".to_owned()),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllowedContent {
    pub format: DetectedFormat,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisallowedContent {
    pub kind: DisallowedKind,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentInspection {
    Allowed(AllowedContent),
    Disallowed(DisallowedContent),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecuredUpload {
    pub safe_filename: String,
    pub content_type: String,
    pub detected_format: DetectedFormat,
    pub size_bytes: usize,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedUpload {
    pub reason: RejectionReason,
    pub message: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadValidation {
    Secured(SecuredUpload),
    Rejected(RejectedUpload),
}

pub fn inspect_content(content: &[u8]) -> ContentInspection {
    if signatures::shebang(content) {
        return ContentInspection::Disallowed(DisallowedContent {
            kind: DisallowedKind::Executable,
        });
    }
    if content.starts_with(b"%PDF-") {
        return ContentInspection::Allowed(AllowedContent {
            format: DetectedFormat::Pdf,
        });
    }
    let is_text = !content.contains(&0) && std::str::from_utf8(content).is_ok();
    if let Some(kind) = signatures::binary_kind(content, is_text) {
        return ContentInspection::Disallowed(DisallowedContent { kind });
    }
    if is_text {
        ContentInspection::Allowed(AllowedContent {
            format: DetectedFormat::Text,
        })
    } else {
        ContentInspection::Disallowed(DisallowedContent {
            kind: DisallowedKind::UnknownBinary,
        })
    }
}
pub fn generate_safe_filename<Id: FnOnce() -> uuid::Uuid>(
    format: DetectedFormat,
    uuid: Id,
) -> String {
    let extension = match format {
        DetectedFormat::Pdf => "pdf",
        DetectedFormat::Text => "txt",
    };
    format!("{}.{}", uuid().simple(), extension)
}

pub fn validate_upload<Id: FnOnce() -> uuid::Uuid>(
    content: &[u8],
    scanner: &dyn MalwareScanner,
    uuid: Id,
    max_size_bytes: usize,
) -> UploadValidation {
    let size = content.len();
    if size == 0 {
        return UploadValidation::Rejected(RejectedUpload {
            reason: RejectionReason::EmptyFile,
            message: "Uploaded file is empty.".to_owned(),
        });
    }
    if size > max_size_bytes {
        return UploadValidation::Rejected(RejectedUpload {
            reason: RejectionReason::FileTooLarge,
            message: format!(
                "Uploaded file is {size} bytes, exceeding the {max_size_bytes}-byte limit."
            ),
        });
    }
    let format = match inspect_content(content) {
        ContentInspection::Disallowed(value) => {
            return UploadValidation::Rejected(rejection::disallowed(value.kind));
        }
        ContentInspection::Allowed(value) => value.format,
    };
    if let Some(rejection) = rejection::scan(scanner.scan(content)) {
        return UploadValidation::Rejected(rejection);
    }
    let content_type = match format {
        DetectedFormat::Pdf => "application/pdf",
        DetectedFormat::Text => "text/plain",
    };
    UploadValidation::Secured(SecuredUpload {
        safe_filename: generate_safe_filename(format, uuid),
        content_type: content_type.to_owned(),
        detected_format: format,
        size_bytes: size,
    })
}

mod rejection;
mod signatures;
mod tests;
