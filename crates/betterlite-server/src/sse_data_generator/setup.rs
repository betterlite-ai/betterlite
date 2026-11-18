use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;

use super::{ItemStream, StreamFailure, StreamItem};
use crate::keepalive_interval::PingSetting;
use crate::stream_keepalive::KeepaliveRouter;
use crate::stream_keepalive::resolver::KeepaliveResolver;
use crate::stream_keepalive::wrapper::{Beat, with_keepalive};

pub(super) struct RequestFlags {
    pub(super) raw: bool,
    pub(super) strip: bool,
    pub(super) skip_done: bool,
}

impl RequestFlags {
    pub(super) fn read(request_data: &JsonObject) -> Self {
        Self {
            raw: matches!(
                request_data.get("_litellm_raw_sse_stream"),
                Some(Json::Bool(true))
            ),
            strip: matches!(
                request_data.get("_litellm_strip_stream_usage"),
                Some(Json::Bool(true))
            ),
            skip_done: matches!(
                request_data.get("_litellm_skip_openai_stream_done"),
                Some(Json::Bool(true))
            ),
        }
    }
}

pub(super) struct BeatSetup<'a> {
    pub(super) items: ItemStream,
    pub(super) router: Option<Arc<dyn KeepaliveRouter>>,
    pub(super) request_data: Arc<JsonObject>,
    pub(super) global: Option<PingSetting>,
    pub(super) hidden: &'a JsonObject,
}

pub(super) fn beats(
    setup: BeatSetup<'_>,
) -> BoxStream<'static, Beat<Result<StreamItem, StreamFailure>>> {
    let mut resolver =
        KeepaliveResolver::new(setup.router.clone(), setup.request_data, setup.global);
    let initial = resolver.resolve(setup.hidden);
    if setup.router.is_some()
        || initial
            .is_sign_positive()
            .then_some(initial)
            .is_some_and(|seconds| seconds != 0.0_f64)
    {
        Box::pin(with_keepalive(setup.items, resolver, initial))
    } else {
        Box::pin(setup.items.map(Beat::Item))
    }
}
