use std::fmt::Display;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use tokio::runtime::{Handle, TryCurrentError};
use tokio::task::{JoinError, JoinHandle};

use crate::pass_through_logging::anthropic_batch::{
    BatchRegistration, ManagedBatchRegistry, RegistryError,
};
use crate::pass_through_logging::background_tasks::BackgroundTasks;

#[must_use]
pub fn optional_str(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

#[must_use]
pub fn is_collection_route(url_route: &str, collection_suffix: &str) -> bool {
    let path = url_route
        .split_once('?')
        .map_or(url_route, |(path, _query)| path);
    path.trim_end_matches('/').ends_with(collection_suffix)
}

#[must_use]
pub fn request_tags_from_metadata(
    request_metadata: &IndexMap<String, Json>,
) -> Option<Vec<String>> {
    if let Some(tags) = sanitized_str_tuple(request_metadata.get("tags"))
        && !tags.is_empty()
    {
        return Some(tags);
    }
    let key_metadata = request_metadata.get("user_api_key_auth_metadata")?;
    let Json::Object(fields) = key_metadata else {
        return None;
    };
    sanitized_str_tuple(fields.get("tags"))
}

// The write and the log of its result are one task, so a registration is never half done when the server stops.
pub(crate) fn spawn_batch_registration(
    background: &BackgroundTasks,
    registry: &Arc<dyn ManagedBatchRegistry>,
    provider: &'static str,
    registration: BatchRegistration,
) -> Result<JoinHandle<()>, TryCurrentError> {
    let unified_object_id = registration.unified_object_id.clone();
    let model_object_id = registration.model_object_id.clone();
    let is_batch_create = registration.create_if_missing;
    // the tracker spawns on a runtime, so a missing one becomes the caller's error instead of a panic
    let _runtime = Handle::try_current()?;
    let stored = registry.store_unified_object_id(registration);
    let consequence = registration_consequence(is_batch_create);
    let warning = format!("{provider} batch registration cancelled at shutdown; {consequence}");
    Ok(background.spawn(
        async move {
            log_batch_registration_result(
                Ok(stored.await),
                provider,
                &unified_object_id,
                &model_object_id,
                is_batch_create,
            );
        },
        warning,
    ))
}

pub fn log_batch_registration_result<E>(
    finished: Result<Result<(), E>, JoinError>,
    provider: &str,
    unified_object_id: &str,
    model_object_id: &str,
    is_batch_create: bool,
) where
    E: Display,
{
    let consequence = registration_consequence(is_batch_create);
    match finished {
        Ok(Ok(())) => {
            tracing::info!(
                "Stored {provider} batch managed object with unified_object_id={unified_object_id}, batch_id={model_object_id}"
            );
        }
        Ok(Err(error)) => {
            tracing::error!(
                "Failed to store {provider} batch managed object with unified_object_id={unified_object_id}, batch_id={model_object_id}; {consequence}: {error}"
            );
        }
        Err(join_error) => {
            let error = join_error_text(&join_error);
            tracing::error!(
                "Failed to store {provider} batch managed object with unified_object_id={unified_object_id}, batch_id={model_object_id}; {consequence}: {error}"
            );
        }
    }
}

const fn registration_consequence(is_batch_create: bool) -> &'static str {
    if is_batch_create {
        "its cost will not be tracked"
    } else {
        "its status and output file may be stale"
    }
}

fn sanitized_str_tuple(value: Option<&Json>) -> Option<Vec<String>> {
    let Some(Json::List(items)) = value else {
        return None;
    };
    Some(
        items
            .iter()
            .filter_map(optional_str)
            .map(strip_null_bytes)
            .collect(),
    )
}

fn strip_null_bytes(text: &str) -> String {
    text.chars()
        .filter(|character| *character != '\0')
        .collect()
}

fn join_error_text(join_error: &JoinError) -> String {
    if join_error.is_cancelled() {
        return "None".to_owned();
    }
    join_error.to_string()
}

mod tests;
