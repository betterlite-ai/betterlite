use std::io;
use std::path::Path;

use tokio::fs;
use toml_edit::DocumentMut;

use super::error::CodexSettingsError;
use super::{CODEX_PROXY_PROVIDER, MODEL_PROVIDERS_KEY};

pub async fn read_settings(settings_path: &Path) -> Result<DocumentMut, CodexSettingsError> {
    let bytes = match fs::read(settings_path).await {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(source) => {
            return Err(CodexSettingsError::Read {
                path: settings_path.to_owned(),
                source,
            });
        }
    };
    let text = String::from_utf8(bytes).map_err(|source| CodexSettingsError::NotUtf8 {
        path: settings_path.to_owned(),
        source,
    })?;
    let document = text
        .parse::<DocumentMut>()
        .map_err(|source| CodexSettingsError::Parse {
            path: settings_path.to_owned(),
            source,
        })?;
    validate(&document)?;
    Ok(document)
}

fn validate(document: &DocumentMut) -> Result<(), CodexSettingsError> {
    let Some(providers) = document.as_table().get(MODEL_PROVIDERS_KEY) else {
        return Ok(());
    };
    let Some(table) = providers.as_table_like() else {
        return Err(CodexSettingsError::ProvidersNotTable);
    };
    match table.get(CODEX_PROXY_PROVIDER) {
        Some(provider) if provider.as_table_like().is_none() => {
            Err(CodexSettingsError::ProviderNotTable)
        }
        Some(_) | None => Ok(()),
    }
}

mod tests;
