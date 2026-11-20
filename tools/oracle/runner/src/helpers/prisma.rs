use std::ffi::OsStr;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::HelperRunError;

pub(super) fn venv_site_packages(project: &Path) -> Result<PathBuf, HelperRunError> {
    let library = project.join(".venv/lib");
    let Some(python) = python_folder(&library) else {
        return Err(HelperRunError::PrismaMissing { path: library });
    };
    Ok(python.join("site-packages"))
}

fn python_folder(library: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(library).ok()?;
    let mut folders = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| name.starts_with("python3."))
        });
    let folder = folders.next()?;
    folders.next().is_none().then_some(folder)
}

// prisma installs its CLI into one cache under the user's home, and concurrent npm installs there corrupt it
pub(super) fn lock_prisma_generate(path: &Path) -> Result<fs::File, HelperRunError> {
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|source| HelperRunError::PrismaLock {
            path: path.to_path_buf(),
            source,
        })?;
    file.lock().map_err(|source| HelperRunError::PrismaLock {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(file)
}

// LiteLLM's key lookup reads a key's models as a list and rejects NULL, and the fixture's seed leaves that column empty
const KEY_MODELS_SQL: &str =
    "UPDATE \"LiteLLM_VerificationToken\" SET models = ARRAY[]::TEXT[] WHERE models IS NULL";

pub fn fill_key_models(root: &Path, database_url: &str) -> Result<(), HelperRunError> {
    let project = root.join("tools/oracle/litellm");
    let schema = venv_site_packages(&project)?.join("litellm/proxy/schema.prisma");
    if !schema.is_file() {
        return Err(HelperRunError::PrismaMissing { path: schema });
    }
    let mut child = Command::new("uv")
        .args([
            "run", "--locked", "prisma", "db", "execute", "--stdin", "--schema",
        ])
        .arg(&schema)
        .env("DATABASE_URL", database_url)
        .current_dir(&project)
        .stdin(Stdio::piped())
        .spawn()
        .map_err(HelperRunError::PrismaExecute)?;
    let mut stdin = child.stdin.take().ok_or(HelperRunError::PrismaStdin)?;
    let written = stdin
        .write_all(KEY_MODELS_SQL.as_bytes())
        .map_err(HelperRunError::PrismaExecute);
    drop(stdin);
    let status = child.wait().map_err(HelperRunError::PrismaExecute)?;
    written?;
    if status.success() {
        Ok(())
    } else {
        Err(HelperRunError::PrismaExecuteExited(status))
    }
}
