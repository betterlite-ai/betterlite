#![cfg(test)]

use std::io::ErrorKind;
use std::path::Path;
use std::time::{Duration, SystemTime};

use crate::credential_sources::files::{FileReader as _, StdFiles};
use crate::credential_sources::sdk::{CredentialClock as _, SystemClock};

#[test]
fn the_std_reader_reads_an_existing_file() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let bytes = StdFiles.read(&manifest).unwrap();
    assert!(
        String::from_utf8(bytes)
            .unwrap()
            .contains("betterlite-vertex")
    );
}

#[test]
fn the_std_reader_reports_a_missing_file() {
    let error = StdFiles
        .read(Path::new("/nonexistent/credentials.json"))
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound);
}

#[test]
fn the_system_clock_reads_the_current_time() {
    let reading = SystemClock.now();
    let elapsed = SystemTime::now().duration_since(reading).unwrap();
    assert!(elapsed < Duration::from_secs(60), "{elapsed:?}");
}
