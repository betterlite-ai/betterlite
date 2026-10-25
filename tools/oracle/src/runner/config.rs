use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str;

use crate::OracleError;

pub(super) fn process_environment_url() -> Result<Option<String>, OracleError> {
    let output = Command::new("printenv")
        .arg("POSTGRES_TEST_URL")
        .output()
        .map_err(OracleError::Environment)?;
    if !output.status.success() {
        return Ok(None);
    }
    let value = str::from_utf8(&output.stdout).map_err(OracleError::EnvironmentText)?;
    Ok(Some(value.trim_end_matches('\n').to_owned()))
}

pub(super) fn with_database(
    base_path: &Path,
    database_url: &str,
    filename: &str,
) -> Result<PathBuf, OracleError> {
    let source = fs::read_to_string(base_path).map_err(|source| OracleError::Filesystem {
        path: base_path.to_owned(),
        source,
    })?;
    let escaped = database_url.replace('\\', "\\\\").replace('"', "\\\"");
    let settings = format!("  database_url: \"{escaped}\"\n  disable_prisma_schema_update: true");
    let config = source
        .lines()
        .position(|line| line.trim() == "general_settings:")
        .and_then(|index| index.checked_add(1))
        .map_or_else(
            || format!("{source}\ngeneral_settings:\n{settings}\n"),
            |index| {
                let mut lines = source.lines().map(str::to_owned).collect::<Vec<_>>();
                lines.insert(index, settings.clone());
                lines.join("\n")
            },
        );
    let path = base_path.with_file_name(filename);
    fs::write(&path, config).map_err(|write_error| OracleError::Filesystem {
        path: path.clone(),
        source: write_error,
    })?;
    Ok(path)
}
