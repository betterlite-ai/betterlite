use betterlite_core::api::video_list_call::{ListRequest, VideoListCall};

use super::super::super::clients::NativeSource;
use super::Parts;

// the request B3340's list transport takes
pub(super) const fn build<'deps, 'env>(
    parts: Parts<'deps, 'env, VideoListCall>,
) -> ListRequest<'deps, 'deps, 'env, NativeSource> {
    ListRequest {
        transport: parts.transport,
        deps: parts.deps,
        call: parts.call,
        client: None,
    }
}
