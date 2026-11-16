pub mod decode;
pub mod error;
pub mod params;
pub mod settings;
mod tests;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::fal::images::error::FalImageError;

// B997 reads a missing or null field as none and refuses any other type, naming the field
pub fn image_url(
    image: &IndexMap<String, Json>,
    model: &'static str,
) -> Result<Option<String>, FalImageError> {
    match image.get("url") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(url)) => Ok(Some(url.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(FalImageError::FieldNotText {
                field: "url",
                model,
            })
        }
    }
}
