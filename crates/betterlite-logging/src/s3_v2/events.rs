use betterlite_config::loader::EnvironmentLookup;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::s3_transport::batch_element::{ContentType, S3BatchLoggingElement};
use crate::s3_transport::naming::{object_download_filename, object_key};
use crate::s3_transport::options::{PartitionGranularity, resolve_log_prompts_only};
use crate::s3_v2::{Feature, PartitionGranularityCache, S3Parameters, redact};

#[derive(Debug, thiserror::Error)]
pub enum EventError {
    #[error("invalid S3 logging payload")]
    Payload(#[from] serde_json::Error),
    #[error("invalid base64 redaction pattern")]
    Redaction(#[from] regex::Error),
}

#[derive(Deserialize)]
struct EventFields {
    id: String,
    metadata: Option<Metadata>,
}

#[derive(Default, Deserialize)]
struct Metadata {
    user_api_key_team_alias: Option<String>,
    user_api_key_alias: Option<String>,
    cold_storage_object_key: Option<String>,
}

pub struct Event<'payload, Payload> {
    pub start_time: NaiveDateTime,
    pub payload: Option<&'payload Payload>,
    pub cold_storage_enabled: bool,
}

pub fn create_batch_element<Config, Payload>(
    parameters: &S3Parameters<Config>,
    event: &Event<'_, Payload>,
    cache: &mut PartitionGranularityCache,
    environment: &dyn EnvironmentLookup,
) -> Result<Option<S3BatchLoggingElement<Payload>>, EventError>
where
    Config: Default,
    Payload: Clone + Serialize + DeserializeOwned,
{
    let Some(input) = event.payload else {
        return Ok(None);
    };
    let encoded = serde_json::to_string(input)?;
    let fields: EventFields = serde_json::from_str(&encoded)?;
    let key = event_key(parameters, event, &fields, cache, environment);
    let prompts_only =
        resolve_log_prompts_only(parameters.connection.log_prompts_only.as_ref(), environment);
    let output = if parameters.features.contains(&Feature::StripBase64) || prompts_only {
        let rewritten = redact::project(&encoded, &parameters.features, prompts_only)?;
        serde_json::from_str(&rewritten)?
    } else {
        input.clone()
    };
    Ok(Some(S3BatchLoggingElement {
        payload: output,
        s3_object_key: key,
        s3_object_download_filename: object_download_filename(event.start_time, &fields.id),
        body: None,
        content_type: ContentType::Json,
        retrying_since: None,
    }))
}

fn prefix_path<Config>(parameters: &S3Parameters<Config>, metadata: &Metadata) -> String
where
    Config: Default,
{
    let mut prefixes = Vec::new();
    if parameters.features.contains(&Feature::TeamPrefix)
        && let Some(team) = metadata
            .user_api_key_team_alias
            .as_ref()
            .filter(|alias| !alias.is_empty())
    {
        prefixes.push(team.as_str());
    }
    if parameters.features.contains(&Feature::KeyPrefix)
        && let Some(key) = metadata
            .user_api_key_alias
            .as_ref()
            .filter(|alias| !alias.is_empty())
    {
        prefixes.push(key.as_str());
    }
    if prefixes.is_empty() {
        String::new()
    } else {
        format!("{}/", prefixes.join("/"))
    }
}

fn event_key<Config, Payload>(
    parameters: &S3Parameters<Config>,
    event: &Event<'_, Payload>,
    fields: &EventFields,
    cache: &mut PartitionGranularityCache,
    environment: &dyn EnvironmentLookup,
) -> String
where
    Config: Default,
{
    let empty = Metadata::default();
    let metadata = fields.metadata.as_ref().unwrap_or(&empty);
    let prefix = prefix_path(parameters, metadata);
    let file_name = format!(
        "time-{}_{}",
        event.start_time.format("%H-%M-%S-%6f"),
        fields.id
    );
    let key_for = |granularity| {
        object_key(
            parameters.connection.path.as_deref().unwrap_or_default(),
            &prefix,
            event.start_time,
            &file_name,
            granularity,
        )
    };
    let cold_key = metadata
        .cold_storage_object_key
        .clone()
        .filter(|candidate| {
            event.cold_storage_enabled
                && (*candidate == key_for(PartitionGranularity::Day)
                    || *candidate == key_for(PartitionGranularity::Hour))
        });
    cold_key.unwrap_or_else(|| {
        key_for(cache.resolve(
            parameters.connection.partition_granularity.as_ref(),
            environment,
        ))
    })
}
