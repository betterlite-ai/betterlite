pub mod annotate;
pub mod compose;
pub mod error;
pub mod grants;
pub mod postgres;
pub mod teams;

mod tests;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

// model_info's text field: these policies compare stored values against names, and no other JSON type matches one.
pub(crate) fn text_field<'info>(info: &'info JsonObject, field: &str) -> Option<&'info str> {
    let Some(Json::Text(text)) = info.get(field) else {
        return None;
    };
    Some(text)
}
