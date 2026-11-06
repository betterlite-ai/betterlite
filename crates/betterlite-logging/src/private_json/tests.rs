#![cfg(test)]

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use rstest::rstest;
use serde_json::Value;

use super::files::{FileAccess, LocalFiles, StagedTemp};
use super::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps, safe_json_loads};

mod private_json_tests_behavior;

fn value(text: &str) -> Value {
    serde_json::from_str(text).expect("valid json")
}

fn content_of(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).expect("reads the file")).expect("valid json")
}

fn mode_of(path: &Path) -> u32 {
    fs::metadata(path)
        .expect("reads metadata")
        .permissions()
        .mode()
        & 0o777
}

fn entries_of(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(directory)
        .expect("reads the directory")
        .map(|entry| {
            entry
                .expect("directory entry")
                .file_name()
                .into_string()
                .expect("a utf-8 file name")
        })
        .collect();
    names.sort_unstable();
    names
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    Write,
    Replace,
}

struct ScriptedFiles {
    fault: Fault,
}

impl FileAccess for ScriptedFiles {
    fn create_parents(&self, directory: &Path) -> io::Result<()> {
        LocalFiles.create_parents(directory)
    }

    fn ensure_private_directory(&self, directory: &Path) -> io::Result<()> {
        LocalFiles.ensure_private_directory(directory)
    }

    fn directory_mode(&self, directory: &Path) -> io::Result<u32> {
        LocalFiles.directory_mode(directory)
    }

    fn set_mode(&self, path: &Path, mode: u32) -> io::Result<()> {
        LocalFiles.set_mode(path, mode)
    }

    fn create_staged(&self, directory: &Path, suffix: Option<&str>) -> io::Result<StagedTemp> {
        LocalFiles.create_staged(directory, suffix)
    }

    fn write_staged(&self, staged: &mut StagedTemp, bytes: &[u8]) -> io::Result<()> {
        if self.fault == Fault::Write {
            return Err(io::Error::other("scripted write failure"));
        }
        LocalFiles.write_staged(staged, bytes)
    }

    fn replace(&self, staged: &Path, target: &Path) -> io::Result<()> {
        if self.fault == Fault::Replace {
            return Err(io::Error::other("scripted replace failure"));
        }
        LocalFiles.replace(staged, target)
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        LocalFiles.remove(path)
    }

    fn rewrite_existing(&self, target: &Path, bytes: &[u8]) -> io::Result<()> {
        LocalFiles.rewrite_existing(target, bytes)
    }
}

#[rstest]
fn replays_the_private_json_corpus(#[files("fuzz/corpus/private_json/*")] path: PathBuf) {
    let bytes = fs::read(&path).expect("corpus file");
    if let Ok(parsed) = safe_json_loads(&bytes) {
        let dumped = safe_dumps(&parsed, DEFAULT_MAX_RECURSE_DEPTH, None).expect("round trip");
        let _restored: Value = serde_json::from_str(&dumped).expect("valid json");
    }
}
