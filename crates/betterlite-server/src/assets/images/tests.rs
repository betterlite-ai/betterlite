#![cfg(test)]

use std::cell::Cell;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;

use rstest::rstest;
use soft_canonicalize::soft_canonicalize;
use tempfile::TempDir;

use super::{AssetLogger, detect_local_image_media_type, resolve_local_image};

#[derive(Default)]
struct RecordingLogger {
    levels: Cell<Vec<&'static str>>,
}

impl RecordingLogger {
    fn levels(&self) -> Vec<&'static str> {
        let levels = self.levels.take();
        let snapshot = levels.clone();
        self.levels.set(levels);
        snapshot
    }

    fn record(&self, level: &'static str) {
        let mut levels = self.levels.take();
        levels.push(level);
        self.levels.set(levels);
    }
}

impl AssetLogger for RecordingLogger {
    fn debug(&self, _message: &str) {
        self.record("debug");
    }

    fn warning(&self, _message: &str) {
        self.record("warning");
    }
}

#[rstest]
#[case::png(b"\x89PNG\r\n\x1a\n".as_slice(), Some("image/png"))]
#[case::gif89a(b"GIF89a".as_slice(), Some("image/gif"))]
#[case::gif_with_b_at_offset_five_is_rejected(b"GIF8bx".as_slice(), None)]
#[case::jpeg(b"\xff\xd8\xff\xe0".as_slice(), Some("image/jpeg"))]
#[case::webp(b"RIFF\x00\x00\x00\x00WEBPVP8 ".as_slice(), Some("image/webp"))]
#[case::ico_variant_one(b"\x00\x00\x01\x00".as_slice(), Some("image/x-icon"))]
#[case::ico_variant_two(b"\x00\x00\x02\x00".as_slice(), Some("image/x-icon"))]
#[case::empty_header(b"".as_slice(), None)]
#[case::unrecognized_signature(b"not an image".as_slice(), None)]
fn detect_local_image_media_type_checks_signatures(
    #[case] header: &[u8],
    #[case] expected: Option<&str>,
) {
    assert_eq!(detect_local_image_media_type(header), expected);
}

#[test]
fn resolve_local_image_returns_the_media_type_for_a_supported_file() {
    let workspace = TempDir::new().unwrap();
    let path = workspace.path().join("logo.png");
    fs::write(&path, b"\x89PNG\r\n\x1a\n").unwrap();
    let logger = RecordingLogger::default();
    let resolved = resolve_local_image(path.to_str().unwrap(), None, &logger).unwrap();
    assert_eq!(resolved.media_type, "image/png");
    assert!(resolved.path.is_file());
    assert!(logger.levels().is_empty());
}

#[test]
fn resolve_local_image_returns_the_realpath_resolved_symlink_target() {
    let workspace = TempDir::new().unwrap();
    let real = workspace.path().join("real");
    fs::create_dir_all(&real).unwrap();
    let target = real.join("image.png");
    fs::write(&target, b"\x89PNG\r\n\x1a\n").unwrap();
    let candidate = workspace.path().join("logo.png");
    symlink(&target, &candidate).unwrap();
    let logger = RecordingLogger::default();
    let resolved = resolve_local_image(candidate.to_str().unwrap(), None, &logger).unwrap();
    assert_eq!(resolved.path, soft_canonicalize(&target).unwrap());
}

#[test]
fn resolve_local_image_returns_none_for_a_missing_file() {
    let logger = RecordingLogger::default();
    let resolved = resolve_local_image("/definitely-missing-betterlite-asset.png", None, &logger);
    assert!(resolved.is_none());
    assert!(logger.levels().is_empty());
}

#[test]
fn resolve_local_image_returns_none_for_an_empty_candidate() {
    let logger = RecordingLogger::default();
    assert!(resolve_local_image("", None, &logger).is_none());
    assert!(logger.levels().is_empty());
}

#[test]
fn resolve_local_image_warns_and_returns_none_for_an_unsupported_format() {
    let workspace = TempDir::new().unwrap();
    let path = workspace.path().join("notes.txt");
    fs::write(&path, b"just text").unwrap();
    let logger = RecordingLogger::default();
    let resolved = resolve_local_image(path.to_str().unwrap(), None, &logger);
    assert!(resolved.is_none());
    assert_eq!(logger.levels(), vec!["warning"]);
}

#[test]
fn resolve_local_image_logs_debug_when_the_file_cannot_be_read() {
    let logger = RecordingLogger::default();
    let resolved = resolve_local_image("/proc/self/mem", None, &logger);
    assert!(resolved.is_none());
    assert_eq!(logger.levels(), vec!["debug"]);
}

#[test]
fn resolve_local_image_expands_a_home_relative_candidate() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("logo.png"), b"\x89PNG\r\n\x1a\n").unwrap();
    let logger = RecordingLogger::default();
    let resolved = resolve_local_image("~/logo.png", Some(workspace.path()), &logger).unwrap();
    assert_eq!(resolved.media_type, "image/png");
}

#[test]
fn resolve_local_image_does_not_expand_another_users_home_directory() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("logo.png"), b"\x89PNG\r\n\x1a\n").unwrap();
    let logger = RecordingLogger::default();
    let resolved = resolve_local_image("~otheruser/logo.png", Some(workspace.path()), &logger);
    assert!(resolved.is_none());
}

#[rstest]
fn replays_corpus(#[files("fuzz/corpus/local_image_header/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let outcome = detect_local_image_media_type(&bytes);
    assert!(outcome.is_none_or(|media_type| !media_type.is_empty()));
}
