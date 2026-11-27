use betterlite_models::decisions::Json;
use serde::Deserialize;

// A declared key the transform reads for presence: Python's `"errors" in response_data` holds for a null value too.
#[derive(Debug, Default, Deserialize)]
#[serde(from = "Json")]
pub enum DeclaredKey {
    #[default]
    Absent,
    Present(Json),
}

impl From<Json> for DeclaredKey {
    fn from(value: Json) -> Self {
        Self::Present(value)
    }
}

// The keys a Nova Canvas reply body declares; other keys carry nothing the transform reads.
#[derive(Debug, Deserialize)]
pub struct CanvasReplyFields {
    pub images: Option<Json>,
    #[serde(default)]
    pub errors: DeclaredKey,
    pub message: Option<Json>,
    pub error: Option<Json>,
}
