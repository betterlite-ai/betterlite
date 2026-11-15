use indexmap::IndexMap;

use crate::gcs_mock::auth::VertexAuth;
use crate::gcs_mock::client::GcsHttpClient;

use super::config_key_behaviors::{ConfigKeyError, config_key};
use super::{GcsBucketLogger, QueuedItem};

// the map keeps first-appearance order, as the dict LiteLLM builds does
pub fn group_items_by_config<Auth, Http>(
    logger: &GcsBucketLogger<Auth, Http>,
    items: &[QueuedItem],
) -> Result<IndexMap<String, Vec<QueuedItem>>, ConfigKeyError>
where
    Auth: VertexAuth + Clone,
    Http: GcsHttpClient,
{
    let mut grouped: IndexMap<String, Vec<QueuedItem>> = IndexMap::new();
    for item in items {
        let key = config_key(logger, item.dynamic_params.as_ref())?;
        grouped.entry(key).or_default().push(item.clone());
    }
    Ok(grouped)
}

mod tests;
