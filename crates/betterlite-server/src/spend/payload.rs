pub mod build;
pub mod cache;
pub mod envelope;
pub mod identifiers;
pub mod metadata;
pub mod source;
pub mod usage;

use std::borrow::Cow;
use std::collections::BTreeSet;

use betterlite_cache::cache_key::error::CacheKeyError;
use betterlite_cache::handler::HandlerError;
use betterlite_config::collector::TracebackPolicy;
use betterlite_config::spend_attribution::Settings as SpendAttributionSettings;
use betterlite_config::spend_storage::MissingSessionId;
use betterlite_db::search_tools::convert::isoformat;
use betterlite_logging::end_user::{EndUserCostTrackingSettings, EndUserIdError};
use betterlite_logging::logging_metadata::decode::DecodeError;
use betterlite_logging::payload::deps::PayloadClock;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::spend_bodies::SpendStoragePolicy;
use betterlite_logging::spend_bodies::session::{SpendTimestamp, ensure_datetime_utc};
use betterlite_logging::spend_error::log_spend_error;
use betterlite_logging::spend_metadata::records::SpendLogsMetadata;
use betterlite_logging::spend_metadata::{
    SpendLogsPayload, SpendMetadataError, merged_spend_metadata, optional_text_field,
};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::price_fields::decimal_lexeme::DecimalLexemeError;
use betterlite_models::resolution::ResolutionContext;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use chrono::TimeZone;
use tokio::sync::watch;

use crate::spend::attribution::AttributionError;
use source::StandardSource;

pub const DEFAULT_CALL_TYPE: &str = "acompletion";
const CREATION_FAILURE: &str = "Error creating spendlogs object - ";

// completion_start_time defaults to the end; a present null stays null instead of raising like the source's astimezone().
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionStart {
    Absent,
    Null,
    At(SpendTimestamp),
}

// get_logging_payload's response_obj: the unknown-model error keeps its identity, and a non-mapping response only fills result.
#[derive(Clone, Copy, Debug)]
pub enum SpendResponse<'request> {
    Absent,
    Mapping(&'request JsonObject),
    Other,
    UnknownModelError,
}

// The router state the payload's attribution reads: the catalog for provider inference and the configured groups.
pub struct SpendRouter<'request> {
    pub catalog: Option<&'request CatalogLookup<'request>>,
    pub settings: &'request SpendAttributionSettings,
    pub resolution: &'request ResolutionContext<'request>,
    pub recognized_groups: &'request BTreeSet<String>,
    pub team_public_names: &'request BTreeSet<String>,
}

// litellm.cache.get_cache_key(**kwargs): only a cache the caller configured derives a key, so a test can inject one.
pub trait SpendCache {
    fn cache_key(&self, kwargs: &JsonObject) -> Result<String, SpendPayloadError>;
}

pub struct PayloadRequest<'request> {
    pub kwargs: &'request JsonObject,
    pub response: SpendResponse<'request>,
    pub start_time: SpendTimestamp,
    pub end_time: SpendTimestamp,
    pub completion_start_time: CompletionStart,
    pub router: Option<&'request SpendRouter<'request>>,
    pub cache: Option<&'request dyn SpendCache>,
    pub clock: &'request dyn PayloadClock,
    pub session_uuid: &'request str,
    pub tracebacks: &'request watch::Receiver<TracebackPolicy>,
}

#[derive(Clone, Copy, Debug)]
pub struct SpendPayloadSettings {
    pub storage: SpendStoragePolicy,
    pub missing_session_id: Option<MissingSessionId>,
    pub end_user: EndUserCostTrackingSettings,
}

#[derive(Debug, thiserror::Error)]
pub enum SpendPayloadError {
    #[error("the {field} field must have the declared type")]
    FieldType { field: &'static str },
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error(transparent)]
    Metadata(#[from] SpendMetadataError),
    #[error(transparent)]
    EndUser(#[from] EndUserIdError),
    #[error(transparent)]
    Attribution(#[from] AttributionError),
    #[error("the {field} timestamp is not in a known time zone")]
    Time { field: &'static str },
    #[error("the cache request could not be read: {source}")]
    CacheRequest {
        #[from]
        source: HandlerError,
    },
    #[error("the cache key could not be derived: {source}")]
    CacheKey {
        #[from]
        source: CacheKeyError,
    },
    #[error("the {record} could not be written as JSON: {source}")]
    Json {
        record: &'static str,
        #[source]
        source: PrivateJsonError,
    },
    #[error("the request cost is not a number: {source}")]
    Cost {
        #[from]
        source: DecimalLexemeError,
    },
}

// Everything get_logging_payload reads from its caller, read and checked once.
pub(crate) struct Context<'request> {
    pub(crate) request: &'request PayloadRequest<'request>,
    pub(crate) kwargs: &'request JsonObject,
    pub(crate) litellm_params: Cow<'request, JsonObject>,
    pub(crate) metadata: JsonObject,
    pub(crate) standard: Option<StandardSource>,
    pub(crate) standard_raw: Option<&'request Json>,
    pub(crate) call_type: Option<&'request str>,
    pub(crate) cache_hit: Option<bool>,
}

impl<'request> Context<'request> {
    fn new(request: &'request PayloadRequest<'request>) -> Result<Self, SpendPayloadError> {
        let kwargs = request.kwargs;
        let litellm_params = params_of(kwargs)?;
        let metadata = merged_spend_metadata(&litellm_params)?;
        Ok(Self {
            request,
            kwargs,
            litellm_params,
            metadata,
            standard: source::standard_source(kwargs.get("standard_logging_object"))?,
            standard_raw: kwargs.get("standard_logging_object"),
            call_type: optional_text_field(kwargs.get("call_type"), "call_type")?,
            cache_hit: cache_hit(kwargs.get("cache_hit"))?,
        })
    }
}

// The identifiers and attribution the record keeps, resolved before the metadata projection.
pub(crate) struct Pieces {
    pub(crate) usage: JsonObject,
    pub(crate) attribution: identifiers::Attribution,
    pub(crate) envelope: envelope::Envelope,
    pub(crate) clean: SpendLogsMetadata,
    pub(crate) request_id: String,
    pub(crate) cache_key: String,
}

impl Pieces {
    fn assemble(
        context: &Context<'_>,
        settings: &SpendPayloadSettings,
    ) -> Result<Self, SpendPayloadError> {
        let usage = usage::project(context)?;
        let attribution = identifiers::resolve(context)?;
        let envelope = envelope::resolve(context, settings)?;
        let mut clean = metadata::project(context, settings, &attribution)?;
        let additional = usage::additional_values(&usage, clean.usage_object.as_ref());
        clean.additional_usage_values = Some(Json::Object(additional));
        let request_id = cache::request_id(context, &attribution);
        let cache_key = cache::key(context, context.request.cache)?;
        Ok(Self {
            usage,
            attribution,
            envelope,
            clean,
            request_id,
            cache_key,
        })
    }
}

pub fn get_logging_payload<T>(
    request: &PayloadRequest<'_>,
    settings: &SpendPayloadSettings,
    zone: &T,
) -> Result<SpendLogsPayload, SpendPayloadError>
where
    T: TimeZone,
{
    let context = Context::new(request)?;
    let pieces = Pieces::assemble(&context, settings)?;
    build::record(&context, settings, zone, &pieces).inspect_err(|error| {
        let message = format!("{CREATION_FAILURE}{error}");
        log_spend_error(request.tracebacks, &message, None, None);
    })
}

#[path = "payload/tests.rs"]
mod tests;

// litellm_params defaults to {} when absent or null, and any other type is named instead of crashing a .get call.
fn params_of(kwargs: &JsonObject) -> Result<Cow<'_, JsonObject>, SpendPayloadError> {
    let absent = JsonObject::new();
    match kwargs.get("litellm_params") {
        None | Some(Json::Null) => Ok(Cow::Owned(absent)),
        Some(Json::Object(fields)) => Ok(Cow::Borrowed(fields)),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(SpendPayloadError::FieldType {
                field: "litellm_params",
            })
        }
    }
}

// kwargs.get("cache_hit", False): absent reads as False and a present null stays None for str().
const fn cache_hit(value: Option<&Json>) -> Result<Option<bool>, SpendPayloadError> {
    match value {
        None => Ok(Some(false)),
        Some(Json::Null) => Ok(None),
        Some(Json::Bool(flag)) => Ok(Some(*flag)),
        Some(Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(SpendPayloadError::FieldType { field: "cache_hit" })
        }
    }
}

// str(cache_hit): Python capitalizes its booleans and writes None.
pub(crate) fn cache_hit_text(value: Option<bool>) -> String {
    match value {
        Some(true) => "True".to_owned(),
        Some(false) => "False".to_owned(),
        None => "None".to_owned(),
    }
}

// The declared counter fields: absent keeps the caller's default and null stays null.
pub(crate) fn counter(
    value: Option<&Json>,
    field: &'static str,
) -> Result<Option<i64>, SpendPayloadError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => number
            .as_i64()
            .map(Some)
            .ok_or(SpendPayloadError::FieldType { field }),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(SpendPayloadError::FieldType { field })
        }
    }
}

// _ensure_datetime_utc over the injected zone, rendered as the isoformat() text the row stores.
pub(crate) fn instant<T>(
    moment: SpendTimestamp,
    zone: &T,
    field: &'static str,
) -> Result<String, SpendPayloadError>
where
    T: TimeZone,
{
    ensure_datetime_utc(moment, zone)
        .map(isoformat)
        .ok_or(SpendPayloadError::Time { field })
}
