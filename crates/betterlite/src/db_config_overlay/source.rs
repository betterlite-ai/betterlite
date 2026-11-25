use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use betterlite_config::sections::includes::{ConfigMap, resolve_includes};
use betterlite_config::sections::settings::JsonValue;
use betterlite_pyhost::json_values::JsonObject;
use betterlite_server::bucket_config::includes::load_config_from_bucket;
use betterlite_server::bucket_config::object::BucketObjectReader;
use betterlite_server::bucket_config::source::BucketConfigSource;
use tokio::fs;

use super::error::OverlayError;

// The bucket reader matches LITELLM_CONFIG_BUCKET_TYPE and is built where the cloud clients are
pub struct ConfigSource {
    pub config_file_path: Option<PathBuf>,
    pub bucket_reader: Option<Arc<dyn BucketObjectReader>>,
}

// get_config's first step: the bucket when LITELLM_CONFIG_BUCKET_NAME is set, else the file, else an empty config
pub async fn read_config(
    source: &ConfigSource,
    env: &BTreeMap<String, String>,
) -> Result<ConfigMap, OverlayError> {
    if let Some(bucket) = BucketConfigSource::from_environment(env)? {
        let Some(reader) = &source.bucket_reader else {
            return Err(OverlayError::NoBucketReader);
        };
        return load_config_from_bucket(&bucket, reader.as_ref())
            .await?
            .ok_or(OverlayError::NothingLoaded);
    }
    let Some(path) = &source.config_file_path else {
        let empty_section = || JsonValue::Object(JsonObject::new());
        return Ok(ConfigMap::from([
            ("model_list".to_owned(), JsonValue::Array(Vec::new())),
            ("general_settings".to_owned(), empty_section()),
            ("router_settings".to_owned(), empty_section()),
            ("litellm_settings".to_owned(), empty_section()),
        ]));
    };
    let config = read_config_file(path).await?;
    Ok(resolve_includes(config, path)?)
}

// _get_config_from_file: yaml.safe_load of the file, where an empty document is refused
async fn read_config_file(path: &Path) -> Result<ConfigMap, OverlayError> {
    let text = fs::read_to_string(path).await.map_err(|source| {
        if source.kind() == ErrorKind::NotFound {
            OverlayError::ConfigFileMissing {
                path: path.to_path_buf(),
            }
        } else {
            OverlayError::ReadConfigFile {
                path: path.to_path_buf(),
                source,
            }
        }
    })?;
    let parsed: JsonValue =
        serde_saphyr::from_str(&text).map_err(|source| OverlayError::ConfigYaml {
            path: path.to_path_buf(),
            source: Box::new(source),
        })?;
    match parsed {
        JsonValue::Object(fields) => Ok(fields.into_iter().collect()),
        JsonValue::Null => Err(OverlayError::ConfigEmpty),
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) | JsonValue::Array(_) => {
            Err(OverlayError::ConfigNotMapping {
                path: path.to_path_buf(),
            })
        }
    }
}
