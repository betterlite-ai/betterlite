use std::path::{Component, Path, PathBuf};
use std::str;

use sha2::{Digest as _, Sha256};
use tokio::fs;

use super::DashboardFixtureError;

const MANIFEST_DIGEST: &str = "c2a1e005f73eda0bb5f5496f90299688872f1e56e71f749b52d81009eae0d5ee";
const FIXTURES: &str = "../../crates/betterlite-db/src/schema_contract/fixtures";

// every file is checked before the first statement runs, so a changed schema never half-applies
pub(super) async fn verified_sql() -> Result<Vec<String>, DashboardFixtureError> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURES);
    let manifest_path = root.join("sql.sha256");
    let manifest = match fs::read(&manifest_path).await {
        Ok(manifest) => manifest,
        Err(source) => {
            return Err(DashboardFixtureError::ManifestRead {
                path: manifest_path,
                source,
            });
        }
    };
    let digest = hex::encode(Sha256::digest(&manifest));
    if digest != MANIFEST_DIGEST {
        return Err(DashboardFixtureError::ManifestDigest { digest });
    }
    let text = str::from_utf8(&manifest)
        .map_err(|source| DashboardFixtureError::ManifestUtf8 { source })?;
    let sql_root = root.join("sql");
    let mut statements = Vec::new();
    for (number, line) in (1_usize..).zip(text.lines()) {
        statements.push(verified_statement(&sql_root, line, number).await?);
    }
    Ok(statements)
}

async fn verified_statement(
    sql_root: &Path,
    line: &str,
    number: usize,
) -> Result<String, DashboardFixtureError> {
    let (expected, file) = line
        .split_once("  ")
        .ok_or(DashboardFixtureError::ManifestLine { line: number })?;
    let path = confined_path(sql_root, file)?;
    let bytes = fs::read(&path)
        .await
        .map_err(|source| DashboardFixtureError::SqlRead {
            file: file.to_owned(),
            source,
        })?;
    let digest = hex::encode(Sha256::digest(&bytes));
    if digest != expected {
        return Err(DashboardFixtureError::SqlDigest {
            file: file.to_owned(),
            digest,
        });
    }
    let text = str::from_utf8(&bytes).map_err(|source| DashboardFixtureError::SqlUtf8 {
        file: file.to_owned(),
        source,
    })?;
    Ok(text.to_owned())
}

pub(super) fn confined_path(sql_root: &Path, file: &str) -> Result<PathBuf, DashboardFixtureError> {
    let mut components = Path::new(file).components();
    let confined =
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none();
    if !confined {
        return Err(DashboardFixtureError::UnsafeFilename {
            name: file.to_owned(),
        });
    }
    Ok(sql_root.join(file))
}
