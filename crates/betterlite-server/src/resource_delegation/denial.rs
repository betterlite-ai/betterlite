use thiserror::Error;

// LiteLLM raises HTTPException(403, detail={"error": detail}); a route turns this into that body.
#[derive(Clone, Debug, Error)]
#[error("{detail}")]
pub struct Denial {
    pub detail: String,
}

impl Denial {
    pub const STATUS_CODE: u16 = 403;
}
