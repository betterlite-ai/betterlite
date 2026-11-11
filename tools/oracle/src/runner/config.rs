use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::fixtures::FixtureGuard;
use crate::OracleError;
use crate::artifacts;
use crate::case::Case;
use crate::config_include_fixtures;
use secrecy::ExposeSecret as _;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
struct ConfigDocument {
    #[serde(default)]
    general_settings: GeneralSettings,
    #[serde(flatten)]
    other: BTreeMap<String, serde_json::Value>,
}

#[derive(Default, Deserialize, Serialize)]
struct GeneralSettings {
    #[serde(default)]
    database_url: Option<String>,
    #[serde(default)]
    disable_prisma_schema_update: Option<bool>,
    #[serde(flatten)]
    other: BTreeMap<String, serde_json::Value>,
}

pub(super) fn prepare_gateway_configs(
    root: &Path,
    first_case: &Case,
    fixtures: &FixtureGuard,
    upstream: &str,
) -> Result<(PathBuf, PathBuf, PathBuf), OracleError> {
    if !first_case.config_files.is_empty() {
        return prepare_include_trees(root, first_case, fixtures, upstream);
    }
    if let Some((first, second)) = &fixtures.databases {
        let first_url = first.options.url.expose_secret();
        let second_url = second.options.url.expose_secret();
        let (group_dir, base_config) = artifacts::prepare(root, first_case, upstream)?;
        let first_config = with_database(&base_config, first_url, "config-first.yaml")?;
        let second_config = with_database(&base_config, second_url, "config-second.yaml")?;
        Ok((group_dir, first_config, second_config))
    } else {
        let (group_dir, config) = artifacts::prepare(root, first_case, upstream)?;
        Ok((group_dir, config.clone(), config))
    }
}

fn prepare_include_trees(
    root: &Path,
    case: &Case,
    fixtures: &FixtureGuard,
    upstream: &str,
) -> Result<(PathBuf, PathBuf, PathBuf), OracleError> {
    let (group_dir, base_config) = artifacts::prepare(root, case, upstream)?;
    let root_text = fs::read_to_string(&base_config).map_err(|source| OracleError::Filesystem {
        path: base_config.clone(),
        source,
    })?;
    let database_urls = fixtures.databases.as_ref().map(|(first, second)| {
        (
            first.options.url.expose_secret(),
            second.options.url.expose_secret(),
        )
    });
    let first_config = write_include_tree(
        &group_dir.join("litellm"),
        &root_text,
        database_urls.map(|urls| urls.0),
        &case.config_files,
        upstream,
    )?;
    let second_config = write_include_tree(
        &group_dir.join("betterlite"),
        &root_text,
        database_urls.map(|urls| urls.1),
        &case.config_files,
        upstream,
    )?;
    Ok((group_dir, first_config, second_config))
}

fn write_include_tree(
    directory: &Path,
    root_text: &str,
    database_url: Option<&str>,
    config_files: &BTreeMap<String, String>,
    upstream: &str,
) -> Result<PathBuf, OracleError> {
    fs::create_dir_all(directory).map_err(|source| OracleError::Filesystem {
        path: directory.to_owned(),
        source,
    })?;
    let path = directory.join("config.yaml");
    let text = match database_url {
        Some(url) => database_document(root_text, url)?,
        None => root_text.to_owned(),
    };
    fs::write(&path, text).map_err(|source| OracleError::Filesystem {
        path: path.clone(),
        source,
    })?;
    config_include_fixtures::write_tree(directory, config_files, upstream)?;
    Ok(path)
}

fn with_database(
    base_path: &Path,
    database_url: &str,
    filename: &str,
) -> Result<PathBuf, OracleError> {
    let text = fs::read_to_string(base_path).map_err(|source| OracleError::Filesystem {
        path: base_path.to_owned(),
        source,
    })?;
    let rendered = database_document(&text, database_url)?;
    let path = base_path.with_file_name(filename);
    fs::write(&path, rendered).map_err(|source| OracleError::Filesystem {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

fn database_document(text: &str, database_url: &str) -> Result<String, OracleError> {
    let mut document: ConfigDocument = serde_saphyr::from_str(text)
        .map_err(|parse_error| OracleError::GeneratedConfig(Box::new(parse_error)))?;
    if document.general_settings.database_url.is_some() {
        return Err(OracleError::DatabaseUrlConflict);
    }
    if document
        .general_settings
        .disable_prisma_schema_update
        .is_some()
    {
        return Err(OracleError::DisableSchemaUpdateConflict);
    }
    document.general_settings.database_url = Some(database_url.to_owned());
    document.general_settings.disable_prisma_schema_update = Some(true);
    serde_saphyr::to_string(&document).map_err(OracleError::GeneratedConfigSerialize)
}

mod tests;
