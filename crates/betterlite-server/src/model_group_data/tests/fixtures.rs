pub mod auth;
pub mod cache;
pub mod router;
pub mod scripted;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

pub fn json_object(pairs: &[(&str, Json)]) -> JsonObject {
    let mut object = JsonObject::new();
    for (key, value) in pairs {
        object.insert((*key).to_owned(), value.clone());
    }
    object
}
