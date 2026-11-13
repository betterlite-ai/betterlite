use crate::logging_metadata::params::LitellmParams;

// the callable cannot ride in JSON metadata, so it already sits in its typed slot
#[must_use]
pub fn scrub_sensitive_metadata(mut params: LitellmParams) -> LitellmParams {
    if let Some(metadata) = params.metadata.as_mut() {
        metadata.shift_remove("langfuse_masking_function");
    }
    params
}

mod tests;
