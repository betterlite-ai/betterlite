use std::path::Path;

use crate::migration_deploy::catalog::{CatalogError, migration_files};

// Mirrors utils.py::_migration_timestamp: Prisma names start with YYYYMMDDHHMMSS; anything else sorts as the oldest
#[must_use]
pub fn migration_timestamp(name: &str) -> i64 {
    let Some(digits) = name.get(..14) else {
        return 0;
    };
    let followed_by_underscore = name.get(14..).is_some_and(|rest| rest.starts_with('_'));
    if !followed_by_underscore || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return 0;
    }
    digits.parse().unwrap_or(0)
}

#[must_use]
pub fn max_migration_timestamp<'name, I>(names: I) -> i64
where
    I: IntoIterator<Item = &'name str>,
{
    let mut maximum = 0;
    for name in names {
        maximum = maximum.max(migration_timestamp(name));
    }
    maximum
}

// Mirrors ProxyExtrasDBManager._get_migration_names: the packaged catalog names under migrations/*/migration.sql
pub async fn get_migration_names(directory: &Path) -> Result<Vec<String>, CatalogError> {
    let catalog = migration_files(directory).await?;
    tracing::info!(
        count = catalog.len(),
        directory = %directory.display(),
        "found migrations"
    );
    Ok(catalog
        .into_iter()
        .map(|migration| migration.name)
        .collect())
}
