use crate::fallback_plans::groups::RequestMetadata;
use crate::fallback_plans::plan::RequestOverrides;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HopRequest<'request> {
    pub model: &'request str,
    pub overrides: RequestOverrides<'request>,
    // the attempt's own copy, so a write during the hop never reaches the outer request
    pub metadata: RequestMetadata,
}

impl<'request> HopRequest<'request> {
    // Ports mid_stream_fallback_hop_kwargs: only a bucket the request carries is copied, the controls pass through.
    #[must_use]
    pub fn for_hop(
        model: &'request str,
        overrides: RequestOverrides<'request>,
        live: &RequestMetadata,
    ) -> Self {
        let mut metadata = RequestMetadata::default();
        let buckets = [
            (&mut metadata.metadata, &live.metadata),
            (&mut metadata.litellm_metadata, &live.litellm_metadata),
        ];
        for (copy, source) in buckets {
            if source.present {
                copy.clone_from(source);
            }
        }
        Self {
            model,
            overrides,
            metadata,
        }
    }
}
