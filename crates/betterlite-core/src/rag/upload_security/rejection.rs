use super::{DisallowedKind, RejectedUpload, RejectionReason, ScanResult, ScanVerdict};

pub(super) fn disallowed(kind: DisallowedKind) -> RejectedUpload {
    let (reason, message) = match kind {
        DisallowedKind::Archive => (
            RejectionReason::ArchiveNotAllowed,
            "Archive uploads are not allowed.",
        ),
        DisallowedKind::Executable => (
            RejectionReason::ExecutableNotAllowed,
            "Executable uploads are not allowed.",
        ),
        DisallowedKind::UnknownBinary => (
            RejectionReason::UnsupportedFormat,
            "Only PDF and UTF-8 text documents are accepted.",
        ),
    };
    RejectedUpload {
        reason,
        message: message.to_owned(),
    }
}

pub(super) fn scan(result: ScanResult) -> Option<RejectedUpload> {
    match result.verdict {
        ScanVerdict::Clean => None,
        ScanVerdict::Infected => Some(RejectedUpload {
            reason: RejectionReason::MalwareDetected,
            message: format!(
                "Uploaded file was flagged by malware scanning ({}).",
                result.signature.as_deref().unwrap_or("unknown signature")
            ),
        }),
        ScanVerdict::Error => Some(RejectedUpload {
            reason: RejectionReason::MalwareScanError,
            message: "Malware scanning could not complete; upload rejected.".to_owned(),
        }),
    }
}
