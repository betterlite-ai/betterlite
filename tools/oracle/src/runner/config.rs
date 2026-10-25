use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::fixtures::FixtureGuard;
use crate::OracleError;
use crate::artifacts;
use crate::case::Case;
use secrecy::ExposeSecret as _;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
struct ConfigDocument {
    #[serde(default)]
    general_settings: GeneralSettings,
    #[serde(flatten)]
    other: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct GeneralSettings {
    #[serde(default)]
    database_url: Option<String>,
    #[serde(default)]
    disable_prisma_schema_update: bool,
    #[serde(flatten)]
    other: BTreeMap<String, serde_json::Value>,
}

pub(super) fn prepare_gateway_configs(
    root: &Path,
    first_case: &Case,
    fixtures: &FixtureGuard,
    upstream: &str,
) -> Result<(PathBuf, PathBuf, PathBuf), OracleError> {
    if let Some((first, second)) = &fixtures.databases {
        let first_url = first.options.database_url();
        let second_url = second.options.database_url();
        let (group_dir, base_config) = artifacts::prepare(root, first_case, upstream)?;
        let first_config =
            with_database(&base_config, first_url.expose_secret(), "config-first.yaml")?;
        let second_config = with_database(
            &base_config,
            second_url.expose_secret(),
            "config-second.yaml",
        )?;
        Ok((group_dir, first_config, second_config))
    } else {
        let (group_dir, config) = artifacts::prepare(root, first_case, upstream)?;
        Ok((group_dir, config.clone(), config))
    }
}

fn with_database(
    base_path: &Path,
    database_url: &str,
    filename: &str,
) -> Result<PathBuf, OracleError> {
    let source = fs::read_to_string(base_path).map_err(|source| OracleError::Filesystem {
        path: base_path.to_owned(),
        source,
    })?;
    let mut config_document: ConfigDocument = serde_saphyr::from_str(&source)
        .map_err(|parse_error| OracleError::GeneratedConfig(Box::new(parse_error)))?;
    if config_document.general_settings.database_url.is_some() {
        return Err(OracleError::DatabaseUrlConflict);
    }
    config_document.general_settings.database_url = Some(database_url.to_owned());
    config_document
        .general_settings
        .disable_prisma_schema_update = true;
    let rendered_config =
        serde_saphyr::to_string(&config_document).map_err(OracleError::GeneratedConfigSerialize)?;
    let path = base_path.with_file_name(filename);
    fs::write(&path, rendered_config).map_err(|write_error| OracleError::Filesystem {
        path: path.clone(),
        source: write_error,
    })?;
    Ok(path)
}
