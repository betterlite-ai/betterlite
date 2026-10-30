use std::path::{Path, PathBuf};

use deadpool_postgres::Client;
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use tokio_postgres::Error as PostgresError;
use uuid::Uuid;

pub mod fixtures;
mod tests;

const LOCK_ID: i64 = 72_707_369;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Migration {
    pub name: String,
    pub checksum: String,
    pub sql: Vec<u8>,
}

pub type MigrationCatalog = Vec<Migration>;

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("could not read migration catalog entry {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Error)]
pub enum DeployError {
    #[error("database operation failed")]
    Database(#[source] PostgresError),
    #[error("migration {name} has an unfinished ledger row")]
    Unfinished { name: String },
    #[error("migration {name} failed")]
    Migration {
        name: String,
        #[source]
        source: PostgresError,
    },
    #[error("migration {name} is not valid UTF-8")]
    InvalidSql { name: String },
}

pub async fn migration_files(
    directory: impl AsRef<Path>,
) -> Result<MigrationCatalog, CatalogError> {
    let migrations = directory.as_ref().join("migrations");
    let mut entries =
        tokio::fs::read_dir(&migrations)
            .await
            .map_err(|source| CatalogError::Read {
                path: migrations.clone(),
                source,
            })?;
    let mut paths = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|source| CatalogError::Read {
            path: migrations.clone(),
            source,
        })?
    {
        let path = entry.path().join("migration.sql");
        match tokio::fs::metadata(&path).await {
            Ok(metadata) if metadata.is_file() => paths.push(path),
            Ok(_) => {}
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(CatalogError::Read { path, source });
            }
        }
    }
    paths.sort();
    let mut catalog = Vec::with_capacity(paths.len());
    for path in paths {
        let sql = tokio::fs::read(&path)
            .await
            .map_err(|source| CatalogError::Read {
                path: path.clone(),
                source,
            })?;
        let checksum = hex::encode(Sha256::digest(&sql));
        let name = path
            .parent()
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            .map(str::to_owned)
            .ok_or_else(|| CatalogError::Read {
                path: path.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "invalid migration name",
                ),
            })?;
        catalog.push(Migration {
            name,
            checksum,
            sql,
        });
    }
    Ok(catalog)
}

pub struct MigrationDeploy {
    pub schema: String,
    pub catalog: MigrationCatalog,
}

impl MigrationDeploy {
    pub async fn run(&self, client: &mut Client) -> Result<(), DeployError> {
        client
            .query_one("SELECT pg_advisory_lock($1)", &[&LOCK_ID])
            .await
            .map_err(DeployError::Database)?;
        let result = self.apply(client).await;
        let unlock = client
            .query_one("SELECT pg_advisory_unlock($1)", &[&LOCK_ID])
            .await;
        match (result, unlock) {
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(DeployError::Database(error)),
            (Ok(()), Ok(_)) => Ok(()),
        }
    }

    async fn apply(&self, client: &mut Client) -> Result<(), DeployError> {
        let ledger = quoted_ledger(&self.schema);
        for migration in &self.catalog {
            let rows = client.query(
                &format!("SELECT id, applied_steps_count, coalesce(logs, ''), finished_at FROM {ledger} WHERE migration_name = $1 AND rolled_back_at IS NULL"),
                &[&migration.name],
            ).await.map_err(DeployError::Database)?;
            if rows.iter().any(|row| {
                row.get::<_, Option<chrono::DateTime<chrono::Utc>>>(3)
                    .is_some()
            }) {
                continue;
            }
            if rows
                .iter()
                .any(|row| !row.get::<_, String>(2).trim().is_empty() || row.get::<_, i32>(1) != 0)
            {
                return Err(DeployError::Unfinished {
                    name: migration.name.clone(),
                });
            }
            let id = Uuid::new_v4().to_string();
            client.execute(&format!("INSERT INTO {ledger} (id, checksum, migration_name, logs, applied_steps_count) VALUES ($1, $2, $3, NULL, 0)"), &[&id, &migration.checksum, &migration.name]).await.map_err(DeployError::Database)?;
            let sql = std::str::from_utf8(&migration.sql).map_err(|_| DeployError::InvalidSql {
                name: migration.name.clone(),
            })?;
            let execution = client.batch_execute(sql).await;
            if let Err(source) = execution {
                let message = source.to_string();
                let _log_result = client
                    .execute(
                        &format!("UPDATE {ledger} SET logs = $2 WHERE id = $1"),
                        &[&id, &message],
                    )
                    .await;
                return Err(DeployError::Migration {
                    name: migration.name.clone(),
                    source,
                });
            }
            client
                .execute(
                    &format!("UPDATE {ledger} SET applied_steps_count = 1 WHERE id = $1"),
                    &[&id],
                )
                .await
                .map_err(DeployError::Database)?;
            client
                .execute(
                    &format!("UPDATE {ledger} SET finished_at = CURRENT_TIMESTAMP WHERE id = $1"),
                    &[&id],
                )
                .await
                .map_err(DeployError::Database)?;
        }
        Ok(())
    }
}

fn quoted_ledger(schema: &str) -> String {
    let escaped = schema.replace('"', "\"\"");
    format!("\"{escaped}\".\"_prisma_migrations\"")
}
