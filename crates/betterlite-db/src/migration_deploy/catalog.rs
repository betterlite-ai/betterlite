use std::io::{Error as IoError, ErrorKind};
use std::path::{Path, PathBuf};

use tokio::fs;

use sha2::{Digest as _, Sha256};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Migration {
    pub name: String,
    pub checksum: String,
    pub sql: Vec<u8>,
}

pub type MigrationCatalog = Vec<Migration>;

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("migration directory name is not valid UTF-8: {path}")]
    Name { path: PathBuf },
    #[error("could not read migration catalog entry {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: IoError,
    },
}

pub async fn migration_files(directory: &Path) -> Result<MigrationCatalog, CatalogError> {
    let migrations = directory.join("migrations");
    let mut entries = match fs::read_dir(&migrations).await {
        Ok(entries) => entries,
        Err(source) if source.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(CatalogError::Read {
                path: migrations,
                source,
            });
        }
    };
    let mut paths = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|source| CatalogError::Read {
            path: migrations.clone(),
            source,
        })?
    {
        if let Some(path) = candidate(&entry.path()).await? {
            paths.push(path);
        }
    }
    paths.sort();
    let mut catalog = Vec::with_capacity(paths.len());
    for path in paths {
        catalog.push(read(&path).await?);
    }
    Ok(catalog)
}

async fn candidate(directory: &Path) -> Result<Option<PathBuf>, CatalogError> {
    let metadata = fs::metadata(directory)
        .await
        .map_err(|source| CatalogError::Read {
            path: directory.to_owned(),
            source,
        })?;
    if !metadata.is_dir() {
        return Ok(None);
    }
    let path = directory.join("migration.sql");
    match fs::symlink_metadata(&path).await {
        Ok(_) => Ok(Some(path)),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(None),
        Err(source) => Err(CatalogError::Read { path, source }),
    }
}

async fn read(path: &Path) -> Result<Migration, CatalogError> {
    let sql = fs::read(path).await.map_err(|source| CatalogError::Read {
        path: path.to_owned(),
        source,
    })?;
    let checksum = hex::encode(Sha256::digest(&sql));
    let name = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .ok_or_else(|| CatalogError::Name {
            path: path.to_owned(),
        })?
        .to_owned();
    Ok(Migration {
        name,
        checksum,
        sql,
    })
}
