use betterlite_core::api::video_remix_call::{RemixRequest, VideoRemixCall};

use super::super::super::clients::NativeSource;
use super::Parts;

// the request B3340's remix transport takes
pub(super) const fn build<'deps, 'env>(
    parts: Parts<'deps, 'env, VideoRemixCall>,
) -> RemixRequest<'deps, 'deps, 'env, NativeSource> {
    RemixRequest {
        transport: parts.transport,
        deps: parts.deps,
        call: parts.call,
        client: None,
    }
}
