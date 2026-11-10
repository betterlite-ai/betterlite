use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::private_json::files::FileAccess;
use serde::{Deserialize, Serialize};

use crate::agent_launch::LITELLM_PROXY_API_KEY_ENV;

use super::{ModelLimits, PI_CONFIG_DIR_ENV, PI_PROVIDER_NAME, PiSyncError};

const OPENAI_COMPLETIONS_API: &str = "openai-completions";
const MODELS_FILE_NAME: &str = "models.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProviderBlock {
    #[serde(rename = "baseUrl")]
    pub base_url: String,
    pub api: &'static str,
    #[serde(rename = "apiKey")]
    pub api_key_reference: String,
    pub models: Vec<ModelEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModelEntry {
    pub id: String,
    #[serde(rename = "contextWindow", skip_serializing_if = "Option::is_none")]
    pub context_window: Option<i64>,
    #[serde(rename = "maxTokens", skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i64>,
}

// pi owns the rest of this file, so those entries stay JSON while the litellm provider is typed
#[derive(Debug, Default, Deserialize, Serialize)]
struct ModelsFile {
    #[serde(default)]
    providers: BTreeMap<String, JsonValue>,
    #[serde(flatten)]
    other: BTreeMap<String, JsonValue>,
}

#[must_use]
pub fn models_json_path(environment: &dyn EnvironmentLookup) -> Option<PathBuf> {
    if let Some(directory) = environment
        .lookup(PI_CONFIG_DIR_ENV)
        .filter(|value| !value.is_empty())
    {
        return Some(Path::new(&directory).join(MODELS_FILE_NAME));
    }
    environment
        .system_home()
        .map(|home| home.join(".pi").join("agent").join(MODELS_FILE_NAME))
}

#[must_use]
pub fn provider_block(
    base_url: &str,
    model_ids: &[String],
    limits: &BTreeMap<String, ModelLimits>,
) -> ProviderBlock {
    ProviderBlock {
        base_url: format!("{}/v1", base_url.trim_end_matches('/')),
        api: OPENAI_COMPLETIONS_API,
        api_key_reference: format!("${LITELLM_PROXY_API_KEY_ENV}"),
        models: model_ids
            .iter()
            .map(|model_id| {
                let found = limits.get(model_id);
                ModelEntry {
                    id: model_id.clone(),
                    context_window: found.and_then(|limit| limit.context_window),
                    max_tokens: found.and_then(|limit| limit.max_tokens),
                }
            })
            .collect(),
    }
}

pub fn sync_models_json(
    path: &Path,
    base_url: &str,
    model_ids: &[String],
    limits: &BTreeMap<String, ModelLimits>,
    files: &dyn FileAccess,
) -> Result<(), PiSyncError> {
    let mut document = read_models_file(path)?;
    let provider =
        serde_json::to_value(provider_block(base_url, model_ids, limits)).map_err(|source| {
            PiSyncError::Encode {
                path: path.to_owned(),
                source,
            }
        })?;
    document
        .providers
        .insert(PI_PROVIDER_NAME.to_owned(), provider);
    let mut contents =
        serde_json::to_vec_pretty(&document).map_err(|source| PiSyncError::Encode {
            path: path.to_owned(),
            source,
        })?;
    contents.push(b'\n');
    // Path("models.json").parent is "" and Path(".").parent is "", both mean the current directory
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    write_models_file(directory, path, &contents, files)
}

fn write_models_file(
    directory: &Path,
    path: &Path,
    contents: &[u8],
    files: &dyn FileAccess,
) -> Result<(), PiSyncError> {
    files
        .create_parents(directory)
        .map_err(|source| PiSyncError::WriteModels {
            path: path.to_owned(),
            source,
        })?;
    let mut staged = files
        .create_staged(directory, Some(".tmp"))
        .map_err(|source| PiSyncError::WriteModels {
            path: path.to_owned(),
            source,
        })?;
    if let Err(source) = files.write_staged(&mut staged, contents) {
        let _unused = files.remove(&staged.path);
        return Err(PiSyncError::WriteModels {
            path: path.to_owned(),
            source,
        });
    }
    if let Err(source) = files.replace(&staged.path, path) {
        let _unused = files.remove(&staged.path);
        return Err(PiSyncError::WriteModels {
            path: path.to_owned(),
            source,
        });
    }
    Ok(())
}

fn read_models_file(path: &Path) -> Result<ModelsFile, PiSyncError> {
    // Path.exists treats every stat failure as absent; only a present file is read
    if fs::metadata(path).is_err() {
        return Ok(ModelsFile::default());
    }
    let contents = fs::read(path).map_err(|source| PiSyncError::ReadModels {
        path: path.to_owned(),
        source,
    })?;
    serde_json::from_slice(&contents).map_err(|source| PiSyncError::ParseModels {
        path: path.to_owned(),
        source,
    })
}
