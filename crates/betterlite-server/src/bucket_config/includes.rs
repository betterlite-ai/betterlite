use std::collections::{BTreeSet, VecDeque};

use betterlite_config::sections::includes::{ConfigMap, include_entries, merge, without_include};
use betterlite_config::sections::settings::settings_rules::JsonValue as Value;

use super::error::BucketConfigError;
use super::include_key::resolve_include_object_key;
use super::object::{BucketObjectReader, parse_bucket_object};
use super::source::BucketConfigSource;

pub async fn resolve_bucket_includes(
    config: ConfigMap,
    object_key: &str,
    reader: &dyn BucketObjectReader,
) -> Result<ConfigMap, BucketConfigError> {
    let mut loaded = BTreeSet::from([object_key.to_owned()]);
    let mut pending = VecDeque::new();
    queue_bucket_includes(&config, object_key, &mut pending)?;
    let mut merged = without_include(config);

    while let Some((entry, declared_in)) = pending.pop_front() {
        let included_key = resolve_include_object_key(&declared_in, &entry);
        if !loaded.insert(included_key.clone()) {
            continue;
        }
        let included = read_included_config(reader, &included_key).await?;
        queue_bucket_includes(&included, &included_key, &mut pending)?;
        merged = merge(merged, without_include(included));
    }
    Ok(merged)
}

pub async fn load_config_from_bucket(
    source: &BucketConfigSource,
    reader: &dyn BucketObjectReader,
) -> Result<Option<ConfigMap>, BucketConfigError> {
    let Some(value) = reader
        .read_object(&source.object_key)
        .await
        .and_then(|bytes| parse_bucket_object(&source.object_key, &bytes))
    else {
        return Ok(None);
    };
    let config = object_to_map(value, &source.object_key)?;
    if config.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        resolve_bucket_includes(config, &source.object_key, reader).await?,
    ))
}

fn queue_bucket_includes(
    config: &ConfigMap,
    declared_in: &str,
    pending: &mut VecDeque<(String, String)>,
) -> Result<(), BucketConfigError> {
    for entry in include_entries(config)? {
        pending.push_back((entry.to_owned(), declared_in.to_owned()));
    }
    Ok(())
}

async fn read_included_config(
    reader: &dyn BucketObjectReader,
    object_key: &str,
) -> Result<ConfigMap, BucketConfigError> {
    let Some(value) = reader
        .read_object(object_key)
        .await
        .and_then(|bytes| parse_bucket_object(object_key, &bytes))
    else {
        return Err(BucketConfigError::IncludedUnreadable {
            object_key: object_key.to_owned(),
        });
    };
    object_to_map(value, object_key)
}

fn object_to_map(value: Value, object_key: &str) -> Result<ConfigMap, BucketConfigError> {
    match value {
        Value::Object(map) => Ok(map.into_iter().collect()),
        Value::Null => Ok(ConfigMap::new()),
        Value::Array(_) | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            Err(BucketConfigError::NotMapping {
                object_key: object_key.to_owned(),
            })
        }
    }
}

mod tests;
