use betterlite_models::decisions::Json;
use serde::{Deserialize, Deserializer};

// A declared key the transform reads for presence: Python's `"errors" in response_data` holds for a null value too.
#[derive(Debug, Default)]
pub enum DeclaredKey {
    #[default]
    Absent,
    Present(Json),
}

impl<'de> Deserialize<'de> for DeclaredKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Json::deserialize(deserializer)?;
        Ok(Self::Present(value))
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
