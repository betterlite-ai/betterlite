use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use betterlite_logging::private_json::files::{FileAccess, LocalFiles, PrivateFiles, StagedTemp};
use secrecy::SecretString;
use tempfile::TempDir;
use toml_edit::{DocumentMut, Item, TableLike};

use crate::codex_settings::configure::{ConfigureCodexSettings, configure_codex_settings};
use crate::codex_settings::error::CodexSettingsError;
use crate::codex_settings::unconfigure::{CodexUnconfigureOutcome, unconfigure_codex_settings};
use crate::codex_settings::version::CodexVersion;

pub(crate) const GATEWAY: &str = "https://gateway.example.com/team";

pub(crate) const KEY: &str = "sk-test";

pub(crate) const MODEL: &str = "gateway-model";

pub(crate) const SAFE_CODEX: &str = "codex-cli 0.129.0";

pub(crate) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

pub(crate) fn document_at(path: &Path) -> DocumentMut {
    fs::read_to_string(path)
        .unwrap()
        .parse::<DocumentMut>()
        .unwrap()
}

pub(crate) fn item_at<'document>(
    document: &'document DocumentMut,
    path: &str,
) -> Option<&'document Item> {
    let mut current: &'document dyn TableLike = document.as_table();
    let mut parts = path.split('.').peekable();
    while let Some(part) = parts.next() {
        let item = current.get(part)?;
        if parts.peek().is_none() {
            return Some(item);
        }
        current = item.as_table_like()?;
    }
    None
}

pub(crate) fn string_at(document: &DocumentMut, path: &str) -> Option<String> {
    item_at(document, path)
        .and_then(Item::as_str)
        .map(str::to_owned)
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn staging_files(directories: &[&Path]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for directory in directories {
        for entry in fs::read_dir(directory).unwrap().flatten() {
            let name = entry.file_name();
            if name.to_str().unwrap().starts_with(".tmp-") {
                found.push(entry.path());
            }
        }
    }
    found
}

pub(crate) async fn configure_with(
    settings_path: &Path,
    files: &PrivateFiles,
    base_url: &str,
    api_key: &str,
    model: &str,
) -> Result<(), CodexSettingsError> {
    let key = secret(api_key);
    let request = ConfigureCodexSettings {
        base_url,
        api_key: &key,
        model,
        settings_path,
        version: CodexVersion::Reported(Some(SAFE_CODEX)),
    };
    configure_codex_settings(&request, files).await
}

pub(crate) async fn unconfigure_at(
    settings_path: &Path,
    files: &PrivateFiles,
) -> Result<CodexUnconfigureOutcome, CodexSettingsError> {
    unconfigure_codex_settings(settings_path, files).await
}

struct FailureRule {
    target: PathBuf,
    occurrence: usize,
}

pub(crate) struct JournalAccess {
    inner: LocalFiles,
    staged: Mutex<Vec<PathBuf>>,
    replaced: Mutex<Vec<PathBuf>>,
    rules: Mutex<Vec<FailureRule>>,
    counts: Mutex<BTreeMap<PathBuf, usize>>,
}

impl Default for JournalAccess {
    fn default() -> Self {
        Self {
            inner: LocalFiles,
            staged: Mutex::new(Vec::new()),
            replaced: Mutex::new(Vec::new()),
            rules: Mutex::new(Vec::new()),
            counts: Mutex::new(BTreeMap::new()),
        }
    }
}

impl JournalAccess {
    pub(crate) fn fail_replace(&self, target: &Path, occurrence: usize) {
        locked(&self.rules).push(FailureRule {
            target: target.to_owned(),
            occurrence,
        });
    }

    pub(crate) fn staged_directories(&self) -> Vec<PathBuf> {
        locked(&self.staged).clone()
    }

    pub(crate) fn committed_paths(&self) -> Vec<PathBuf> {
        locked(&self.replaced).clone()
    }

    fn should_fail(&self, target: &Path) -> bool {
        let mut counts = locked(&self.counts);
        let seen = counts.entry(target.to_owned()).or_insert(0);
        *seen = seen.saturating_add(1);
        let occurrence = *seen;
        drop(counts);
        locked(&self.rules)
            .iter()
            .any(|rule| rule.target == target && rule.occurrence == occurrence)
    }
}

impl FileAccess for JournalAccess {
    fn create_parents(&self, directory: &Path) -> io::Result<()> {
        self.inner.create_parents(directory)
    }

    fn ensure_private_directory(&self, directory: &Path) -> io::Result<()> {
        self.inner.ensure_private_directory(directory)
    }

    fn directory_mode(&self, directory: &Path) -> io::Result<u32> {
        self.inner.directory_mode(directory)
    }

    fn set_mode(&self, path: &Path, mode: u32) -> io::Result<()> {
        self.inner.set_mode(path, mode)
    }

    fn create_staged(&self, directory: &Path, suffix: Option<&str>) -> io::Result<StagedTemp> {
        locked(&self.staged).push(directory.to_owned());
        self.inner.create_staged(directory, suffix)
    }

    fn write_staged(&self, staged: &mut StagedTemp, bytes: &[u8]) -> io::Result<()> {
        self.inner.write_staged(staged, bytes)
    }

    fn replace(&self, staged: &Path, target: &Path) -> io::Result<()> {
        locked(&self.replaced).push(target.to_owned());
        if self.should_fail(target) {
            return Err(io::Error::other("injected commit failure"));
        }
        self.inner.replace(staged, target)
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        self.inner.remove(path)
    }

    fn rewrite_existing(&self, target: &Path, bytes: &[u8]) -> io::Result<()> {
        self.inner.rewrite_existing(target, bytes)
    }
}

pub(crate) fn journal_files() -> (PrivateFiles, Arc<JournalAccess>) {
    let handle = Arc::new(JournalAccess::default());
    let shared: Arc<JournalAccess> = Arc::clone(&handle);
    let access: Arc<dyn FileAccess> = shared;
    (PrivateFiles::from(access), handle)
}

#[cfg(unix)]
pub(crate) fn write_script(home: &TempDir, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;

    let path = home.path().join(name);
    fs::write(&path, body).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
pub(crate) fn file_mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;

    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[cfg(unix)]
pub(crate) fn directory_mode(path: &Path) -> u32 {
    file_mode(path)
}
