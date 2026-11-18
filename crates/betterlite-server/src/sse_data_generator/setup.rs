use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::settings::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;

use super::types::{ErrorStateSource, Flag, ItemStream, StreamCleanup, StreamFailure, StreamHooks};
use crate::keepalive_interval::{PingSetting, coerce_keepalive_interval};
use crate::stream_chunks::StreamItem;
use crate::stream_chunks::fallback::{
    FallbackMeta, client_requested_model, fallback_metadata, include_fallback_errors,
};
use crate::stream_chunks::restamp::RouterCheck;
use crate::stream_keepalive::resolver::KeepaliveResolver;
use crate::stream_keepalive::wrapper::{Beat, KeepaliveSource, keepalive_object, with_keepalive};
use crate::stream_keepalive::{JsonObject as KeepaliveObject, KeepaliveRouter};

pub(super) type BeatStream = BoxStream<'static, Beat<Result<StreamItem, StreamFailure>>>;

pub(super) struct Ports<Key>
where
    Key: Sync,
{
    pub(super) hooks: Arc<dyn StreamHooks<Key>>,
    pub(super) error_state: Arc<dyn ErrorStateSource>,
    pub(super) cleanup: Arc<dyn StreamCleanup<Key>>,
    pub(super) router: Option<Arc<dyn KeepaliveRouter>>,
    pub(super) global_keepalive: Option<PingSetting>,
    pub(super) expose_fallback_errors: Flag,
    pub(super) is_model_router: RouterCheck,
}

impl<Key> Ports<Key>
where
    Key: Sync,
{
    pub(super) fn wrapped_items(
        &self,
        items: ItemStream,
        key: &Key,
        request_data: &JsonObject,
        wrapped: Flag,
    ) -> ItemStream {
        if wrapped.0 {
            return self.hooks.wrap_iterator(items, key, request_data);
        }
        items
    }
}

impl KeepaliveSource for Result<StreamItem, StreamFailure> {
    fn stream_hidden_params(&self) -> Option<&JsonObject> {
        self.as_ref().ok().and_then(StreamItem::hidden_params)
    }
}

#[derive(Debug, Clone)]
pub(super) struct Setup {
    pub(super) raw: Flag,
    pub(super) strip: Flag,
    pub(super) skip_done: Flag,
    pub(super) include: Flag,
    pub(super) wrapped: Flag,
    pub(super) per_chunk: Flag,
    pub(super) requested: String,
    pub(super) fallback: FallbackMeta,
    pub(super) is_model_router: RouterCheck,
}

impl Setup {
    pub(super) fn read<Key>(
        hooks: &dyn StreamHooks<Key>,
        expose: Flag,
        request_data: &JsonObject,
        hidden: &JsonObject,
        is_model_router: RouterCheck,
    ) -> Self
    where
        Key: Sync,
    {
        Self {
            raw: Flag(matches!(
                request_data.get("_litellm_raw_sse_stream"),
                Some(Json::Bool(true))
            )),
            strip: Flag(matches!(
                request_data.get("_litellm_strip_stream_usage"),
                Some(Json::Bool(true))
            )),
            skip_done: Flag(matches!(
                request_data.get("_litellm_skip_openai_stream_done"),
                Some(Json::Bool(true))
            )),
            include: Flag(include_fallback_errors(expose.0, request_data)),
            wrapped: Flag(hooks.needs_iterator_wrap()),
            per_chunk: Flag(hooks.needs_per_chunk_hook()),
            requested: client_requested_model(request_data),
            fallback: fallback_metadata(hidden),
            is_model_router,
        }
    }
}

pub(super) fn list_beats(
    router: Option<Arc<dyn KeepaliveRouter>>,
    global: Option<PingSetting>,
    request_data: Arc<KeepaliveObject>,
    hidden: &JsonObject,
    items: ItemStream,
) -> BeatStream {
    let mut resolver =
        KeepaliveResolver::new(router.as_ref().map(Arc::clone), request_data, global);
    let initial = resolver.resolve(&keepalive_object(hidden));
    let interval = coerce_keepalive_interval(Some(&PingSetting::Seconds(initial)));
    if router.is_some() || interval.is_some() {
        return Box::pin(with_keepalive(items, resolver, initial));
    }
    Box::pin(items.map(Beat::Item))
}

pub(super) fn request_object(data: &BTreeMap<String, JsonValue>) -> JsonObject {
    data.iter()
        .map(|(key, value)| (key.clone(), json_from(value)))
        .collect()
}

fn json_from(value: &JsonValue) -> Json {
    match value {
        JsonValue::Null => Json::Null,
        JsonValue::Bool(flag) => Json::Bool(*flag),
        JsonValue::Number(number) => Json::Number(number.clone()),
        JsonValue::String(text) => Json::Text(text.clone()),
        JsonValue::Array(items) => Json::List(items.iter().map(json_from).collect()),
        JsonValue::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(key, field)| (key.clone(), json_from(field)))
                .collect(),
        ),
    }
}
