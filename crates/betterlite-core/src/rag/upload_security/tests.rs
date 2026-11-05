#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{
    AllowedContent, ContentInspection, DetectedFormat, DisallowedContent, DisallowedKind,
    EICAR_TEST_SIGNATURE, EicarTestMalwareScanner, MAX_UPLOAD_SIZE_BYTES, MalwareScanner,
    RejectedUpload, RejectionReason, ScanResult, ScanVerdict, UploadValidation,
    generate_safe_filename, inspect_content, validate_upload,
};

struct Scanner(ScanVerdict);
impl MalwareScanner for Scanner {
    fn scan(&self, _content: &[u8]) -> ScanResult {
        ScanResult {
            verdict: self.0,
            signature: None,
        }
    }
}
const ID: uuid::Uuid = uuid::Uuid::from_u128(0x550e_8400_e29b_41d4_a716_4466_5544_0000);

#[test]
fn empty_file() {
    assert!(matches!(
        validate_upload(
            b"",
            &Scanner(ScanVerdict::Clean),
            || ID,
            MAX_UPLOAD_SIZE_BYTES
        ),
        UploadValidation::Rejected(RejectedUpload {
            reason: RejectionReason::EmptyFile,
            ..
        })
    ));
}
#[test]
fn oversized_file() {
    assert!(matches!(
        validate_upload(b"xx", &Scanner(ScanVerdict::Clean), || ID, 1),
        UploadValidation::Rejected(RejectedUpload {
            reason: RejectionReason::FileTooLarge,
            ..
        })
    ));
}
#[test]
fn zip_archive() {
    assert!(matches!(
        inspect_content(b"PK\x03\x04stuff"),
        ContentInspection::Disallowed(DisallowedContent {
            kind: DisallowedKind::Archive
        })
    ));
}
#[test]
fn gzip_archive() {
    assert!(matches!(
        inspect_content(b"\x1f\x8b"),
        ContentInspection::Disallowed(DisallowedContent {
            kind: DisallowedKind::Archive
        })
    ));
}
#[test]
fn tar_archive() {
    let mut content = [0; 262];
    content[257..262].copy_from_slice(b"ustar");
    assert!(matches!(
        inspect_content(&content),
        ContentInspection::Disallowed(DisallowedContent {
            kind: DisallowedKind::Archive
        })
    ));
}
#[test]
fn elf_executable() {
    assert!(matches!(
        inspect_content(b"\x7fELF"),
        ContentInspection::Disallowed(DisallowedContent {
            kind: DisallowedKind::Executable
        })
    ));
}
#[test]
fn pe_executable() {
    assert!(matches!(
        inspect_content(b"MZ\0\xff"),
        ContentInspection::Disallowed(DisallowedContent {
            kind: DisallowedKind::Executable
        })
    ));
}
#[test]
fn shebang_script() {
    assert!(matches!(
        inspect_content(b"\xef\xbb\xbf  #!/bin/sh"),
        ContentInspection::Disallowed(DisallowedContent {
            kind: DisallowedKind::Executable
        })
    ));
}
#[test]
fn unknown_binary() {
    assert!(matches!(
        inspect_content(b"\0\xff"),
        ContentInspection::Disallowed(DisallowedContent {
            kind: DisallowedKind::UnknownBinary
        })
    ));
}
#[test]
fn pdf_allowed() {
    assert_eq!(
        inspect_content(b"%PDF-1.7"),
        ContentInspection::Allowed(AllowedContent {
            format: DetectedFormat::Pdf
        })
    );
}
#[test]
fn utf8_text_allowed() {
    assert_eq!(
        inspect_content("hello \u{3bb}".as_bytes()),
        ContentInspection::Allowed(AllowedContent {
            format: DetectedFormat::Text
        })
    );
}
#[test]
fn inspect_content_classifies() {
    assert!(matches!(
        inspect_content(b"#!%PDF-"),
        ContentInspection::Disallowed(DisallowedContent {
            kind: DisallowedKind::Executable
        })
    ));
}
#[test]
fn unique_safe_filenames() {
    assert_eq!(
        generate_safe_filename(DetectedFormat::Pdf, || ID),
        "550e8400e29b41d4a716446655440000.pdf"
    );
    assert_ne!(
        generate_safe_filename(DetectedFormat::Pdf, || ID),
        generate_safe_filename(DetectedFormat::Pdf, || {
            uuid::Uuid::from_u128(0x550e_8400_e29b_41d4_a716_4466_5544_0001)
        })
    );
}
#[test]
fn malware_detected() {
    assert!(matches!(
        validate_upload(
            b"hello",
            &Scanner(ScanVerdict::Infected),
            || ID,
            MAX_UPLOAD_SIZE_BYTES
        ),
        UploadValidation::Rejected(RejectedUpload {
            reason: RejectionReason::MalwareDetected,
            ..
        })
    ));
}
#[test]
fn scan_error_fails_closed() {
    assert!(matches!(
        validate_upload(
            b"hello",
            &Scanner(ScanVerdict::Error),
            || ID,
            MAX_UPLOAD_SIZE_BYTES
        ),
        UploadValidation::Rejected(RejectedUpload {
            reason: RejectionReason::MalwareScanError,
            ..
        })
    ));
}
#[test]
fn clean_scanner_allows() {
    assert!(matches!(
        validate_upload(
            b"hello",
            &Scanner(ScanVerdict::Clean),
            || ID,
            MAX_UPLOAD_SIZE_BYTES
        ),
        UploadValidation::Secured(_)
    ));
}
#[test]
fn eicar_default_clean() {
    assert_eq!(
        EicarTestMalwareScanner.scan(b"hello").verdict,
        ScanVerdict::Clean
    );
}
#[test]
fn eicar_scanned_infected() {
    assert_eq!(
        EicarTestMalwareScanner.scan(EICAR_TEST_SIGNATURE).verdict,
        ScanVerdict::Infected
    );
}
#[test]
fn eicar_text_classification() {
    assert_eq!(
        inspect_content(EICAR_TEST_SIGNATURE),
        ContentInspection::Allowed(AllowedContent {
            format: DetectedFormat::Text
        })
    );
}

#[test]
fn eicar_format_passes_then_scanner_blocks() {
    assert!(matches!(
        validate_upload(
            EICAR_TEST_SIGNATURE,
            &Scanner(ScanVerdict::Clean),
            || ID,
            MAX_UPLOAD_SIZE_BYTES
        ),
        UploadValidation::Secured(_)
    ));
    assert!(matches!(
        validate_upload(
            EICAR_TEST_SIGNATURE,
            &EicarTestMalwareScanner,
            || ID,
            MAX_UPLOAD_SIZE_BYTES
        ),
        UploadValidation::Rejected(RejectedUpload {
            reason: RejectionReason::MalwareDetected,
            ..
        })
    ));
}

#[rstest::rstest]
fn replays_the_upload_security_content_corpus(
    #[files("fuzz/corpus/upload_security_content/*")] path: PathBuf,
) {
    let content = fs::read(path).unwrap();
    if let ContentInspection::Allowed(allowed) = inspect_content(&content) {
        match allowed.format {
            DetectedFormat::Pdf => assert!(content.starts_with(b"%PDF-")),
            DetectedFormat::Text => {
                assert!(!content.contains(&0));
                assert_eq!(
                    str::from_utf8(&content).map(str::as_bytes),
                    Ok(content.as_slice())
                );
            }
        }
    }
}

#[rstest::rstest]
#[case::zip_empty(b"PK\x05\x06", DisallowedKind::Archive)]
#[case::zip_spanned(b"PK\x07\x08", DisallowedKind::Archive)]
#[case::xz(b"\xfd7zXZ\0", DisallowedKind::Archive)]
#[case::seven_zip(b"7z\xbc\xaf'\x1c", DisallowedKind::Archive)]
#[case::rar4(b"Rar!\x1a\x07\0", DisallowedKind::Archive)]
#[case::rar5(b"Rar!\x1a\x07\x01\0", DisallowedKind::Archive)]
#[case::lz4(b"\x04\x22\x4d\x18", DisallowedKind::Archive)]
#[case::zstd(b"\x28\xb5\x2f\xfd", DisallowedKind::Archive)]
#[case::bzip(b"BZh\xff", DisallowedKind::Archive)]
#[case::java(b"\xca\xfe\xba\xbe", DisallowedKind::Executable)]
#[case::mach32(b"\xfe\xed\xfa\xce", DisallowedKind::Executable)]
#[case::mach64(b"\xfe\xed\xfa\xcf", DisallowedKind::Executable)]
#[case::mach32_reverse(b"\xce\xfa\xed\xfe", DisallowedKind::Executable)]
#[case::mach64_reverse(b"\xcf\xfa\xed\xfe", DisallowedKind::Executable)]
#[case::wasm(b"\0asm", DisallowedKind::Executable)]
#[case::dex(b"dex\n\0", DisallowedKind::Executable)]
#[case::vertical_tab_shebang(b"\x0b#!", DisallowedKind::Executable)]
#[case::nul(b"text\0", DisallowedKind::UnknownBinary)]
#[case::truncated_utf8(b"\xe2\x82", DisallowedKind::UnknownBinary)]
fn additional_signatures(#[case] content: &[u8], #[case] kind: DisallowedKind) {
    assert_eq!(
        inspect_content(content),
        ContentInspection::Disallowed(DisallowedContent { kind })
    );
}

#[rstest::rstest]
#[case::bzip_text(b"BZh plain text")]
#[case::pe_text(b"MZ plain text")]
#[case::dex_text(b"dex\nplain text")]
#[case::empty(b"")]
#[case::bom_not_at_start(b" \xef\xbb\xbf#!")]
fn ambiguous_text_allowed(#[case] content: &[u8]) {
    assert_eq!(
        inspect_content(content),
        ContentInspection::Allowed(AllowedContent {
            format: DetectedFormat::Text
        })
    );
}

#[test]
fn pdf_precedes_binary_magic() {
    let mut content = [0; 262];
    content[..5].copy_from_slice(b"%PDF-");
    content[257..262].copy_from_slice(b"ustar");
    assert_eq!(
        inspect_content(&content),
        ContentInspection::Allowed(AllowedContent {
            format: DetectedFormat::Pdf
        })
    );
    assert_eq!(
        inspect_content(&content[..261]),
        ContentInspection::Allowed(AllowedContent {
            format: DetectedFormat::Pdf
        })
    );
}

struct CountingScanner(AtomicUsize);
impl MalwareScanner for CountingScanner {
    fn scan(&self, content: &[u8]) -> ScanResult {
        assert_eq!(content, b"hello");
        self.0.fetch_add(1, Ordering::SeqCst);
        ScanResult {
            verdict: ScanVerdict::Clean,
            signature: None,
        }
    }
}

#[rstest::rstest]
#[case::empty(b"", 0)]
#[case::too_large(b"hello", 4)]
#[case::archive(b"PK\x03\x04", MAX_UPLOAD_SIZE_BYTES)]
#[case::executable(b"#!/bin/sh", MAX_UPLOAD_SIZE_BYTES)]
#[case::binary(b"\0", MAX_UPLOAD_SIZE_BYTES)]
fn rejection_skips_dependencies(#[case] content: &[u8], #[case] max: usize) {
    let scanner = CountingScanner(AtomicUsize::new(0));
    let mut uuid_calls = 0_usize;
    let result = validate_upload(
        content,
        &scanner,
        || {
            uuid_calls = uuid_calls.checked_add(1).unwrap();
            ID
        },
        max,
    );
    assert!(matches!(result, UploadValidation::Rejected(_)));
    assert_eq!(uuid_calls, 0);
    assert_eq!(scanner.0.load(Ordering::SeqCst), 0);
}

#[test]
fn size_boundary_calls_dependencies_once() {
    assert_eq!(MAX_UPLOAD_SIZE_BYTES, 0x2000_0000);
    let scanner = CountingScanner(AtomicUsize::new(0));
    let mut uuid_calls = 0_usize;
    let result = validate_upload(
        b"hello",
        &scanner,
        || {
            uuid_calls = uuid_calls.checked_add(1).unwrap();
            ID
        },
        5,
    );
    assert_eq!(
        result,
        UploadValidation::Secured(super::SecuredUpload {
            safe_filename: "550e8400e29b41d4a716446655440000.txt".to_owned(),
            content_type: "text/plain".to_owned(),
            detected_format: DetectedFormat::Text,
            size_bytes: 5,
        })
    );
    assert_eq!(uuid_calls, 1);
    assert_eq!(scanner.0.load(Ordering::SeqCst), 1);
}

#[test]
fn eicar_substring_and_partial() {
    let mut embedded = b"prefix".to_vec();
    embedded.extend_from_slice(EICAR_TEST_SIGNATURE);
    embedded.extend_from_slice(b"suffix");
    assert_eq!(
        EicarTestMalwareScanner.scan(&embedded),
        ScanResult {
            verdict: ScanVerdict::Infected,
            signature: Some("EICAR-STANDARD-ANTIVIRUS-TEST-FILE".to_owned())
        }
    );
    assert_eq!(
        EicarTestMalwareScanner.scan(&EICAR_TEST_SIGNATURE[1..]),
        ScanResult {
            verdict: ScanVerdict::Clean,
            signature: None
        }
    );
}

#[rstest::rstest]
#[case::unknown(None, "unknown signature")]
#[case::named(Some("Test.Sig"), "Test.Sig")]
#[case::explicit_empty(Some(""), "")]
fn infected_signature_message(#[case] signature: Option<&str>, #[case] expected: &str) {
    assert_eq!(
        super::rejection::scan(&ScanResult {
            verdict: ScanVerdict::Infected,
            signature: signature.map(str::to_owned),
        }),
        Some(RejectedUpload {
            reason: RejectionReason::MalwareDetected,
            message: format!("Uploaded file was flagged by malware scanning ({expected})."),
        })
    );
}

#[rstest::rstest]
#[case::infected(ScanVerdict::Infected)]
#[case::error(ScanVerdict::Error)]
fn failed_scan_skips_uuid(#[case] verdict: ScanVerdict) {
    let mut calls = 0_usize;
    let result = validate_upload(
        b"hello",
        &Scanner(verdict),
        || {
            calls = calls.checked_add(1).unwrap();
            ID
        },
        MAX_UPLOAD_SIZE_BYTES,
    );
    assert!(matches!(result, UploadValidation::Rejected(_)));
    assert_eq!(calls, 0);
}
