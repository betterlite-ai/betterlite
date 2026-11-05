use betterlite_core::rag::upload_security::{
    self, EicarTestMalwareScanner, MalwareScanner, ScanResult, ScanVerdict,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) content_hex: String,
    #[serde(default = "default_verdict")]
    pub(crate) scanner: ScannerKind,
    #[serde(default)]
    pub(crate) signature: Option<String>,
    #[serde(default = "default_size")]
    pub(crate) max_size_bytes: usize,
    #[serde(default = "default_uuid")]
    pub(crate) uuid: String,
}
const fn default_verdict() -> ScannerKind {
    ScannerKind::Clean
}
const fn default_size() -> usize {
    upload_security::MAX_UPLOAD_SIZE_BYTES
}
fn default_uuid() -> String {
    "550e8400-e29b-41d4-a716-446655440000".to_owned()
}

pub(crate) struct Op;
impl crate::helpers::Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let content = hex::decode(&input.content_hex).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "ValueError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        let id = uuid::Uuid::parse_str(&input.uuid).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "ValueError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        let scanner = MatchScanner {
            verdict: input.scanner,
            signature: input.signature.clone(),
        };
        let result =
            upload_security::validate_upload(&content, &scanner, || id, input.max_size_bytes);
        output(result)
    }
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum Output {
    Secured {
        safe_filename: String,
        content_type: String,
        detected_format: &'static str,
        size_bytes: usize,
    },
    Rejected {
        reason: &'static str,
        message: String,
    },
}

fn output(result: upload_security::UploadValidation) -> crate::helpers::HelperResult {
    let value = match result {
        upload_security::UploadValidation::Secured(upload) => Output::Secured {
            safe_filename: upload.safe_filename,
            content_type: upload.content_type,
            detected_format: match upload.detected_format {
                upload_security::DetectedFormat::Pdf => "pdf",
                upload_security::DetectedFormat::Text => "text",
            },
            size_bytes: upload.size_bytes,
        },
        upload_security::UploadValidation::Rejected(rejection) => Output::Rejected {
            reason: reason_name(rejection.reason),
            message: rejection.message,
        },
    };
    serde_json::to_value(value).map_err(|error| {
        Box::new(crate::helpers::HelperError {
            class: "SerializationError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ScannerKind {
    Clean,
    Infected,
    Error,
    Eicar,
}
struct MatchScanner {
    verdict: ScannerKind,
    signature: Option<String>,
}
impl MalwareScanner for MatchScanner {
    fn scan(&self, content: &[u8]) -> ScanResult {
        let verdict = match self.verdict {
            ScannerKind::Clean => ScanVerdict::Clean,
            ScannerKind::Infected => ScanVerdict::Infected,
            ScannerKind::Error => ScanVerdict::Error,
            ScannerKind::Eicar => return EicarTestMalwareScanner.scan(content),
        };
        ScanResult {
            verdict,
            signature: self.signature.clone(),
        }
    }
}
const fn reason_name(reason: upload_security::RejectionReason) -> &'static str {
    match reason {
        upload_security::RejectionReason::EmptyFile => "empty_file",
        upload_security::RejectionReason::FileTooLarge => "file_too_large",
        upload_security::RejectionReason::ArchiveNotAllowed => "archive_not_allowed",
        upload_security::RejectionReason::ExecutableNotAllowed => "executable_not_allowed",
        upload_security::RejectionReason::UnsupportedFormat => "unsupported_format",
        upload_security::RejectionReason::MalwareDetected => "malware_detected",
        upload_security::RejectionReason::MalwareScanError => "malware_scan_error",
    }
}
