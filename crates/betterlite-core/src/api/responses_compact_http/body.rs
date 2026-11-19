use betterlite_prompts::arguments::json::Object;

// LiteLLM's _signed_body_kwargs: non-null signed bytes, even empty ones, are the whole wire body
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignedBody<'data> {
    Data(&'data [u8]),
    Json(&'data Object),
}

#[must_use]
pub const fn signed_body_kwargs<'data>(
    signed_body: Option<&'data [u8]>,
    data: &'data Object,
) -> SignedBody<'data> {
    match signed_body {
        Some(bytes) => SignedBody::Data(bytes),
        None => SignedBody::Json(data),
    }
}
