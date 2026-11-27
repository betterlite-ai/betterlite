use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

// The provider response shapes _replace_file_id_in_response rewrites, each borrowed in place.
pub enum FileResponse<'a> {
    // a dict response: every id and file_id key that is present is rewritten, nested objects are not
    Fields(&'a mut JsonObject),
    // an object carrying both attributes: LiteLLM checks id first, so file_id keeps the provider id
    Both {
        id: &'a mut String,
        file_id: &'a mut String,
    },
    Identifier(&'a mut String),
    FileIdentifier(&'a mut String),
}

// _replace_file_id_in_response: a null response stays null, a dict rewrites both keys, and an object rewrites id first.
pub fn restore_file_identifiers(response: Option<FileResponse<'_>>, original_file_id: &str) {
    let Some(response) = response else {
        return;
    };
    match response {
        FileResponse::Fields(fields) => {
            for name in ["id", "file_id"] {
                if fields.contains_key(name) {
                    fields.insert(name.to_owned(), Json::Text(original_file_id.to_owned()));
                }
            }
        }
        FileResponse::Both { id, .. } => *id = original_file_id.to_owned(),
        FileResponse::Identifier(id) => *id = original_file_id.to_owned(),
        FileResponse::FileIdentifier(file_id) => *file_id = original_file_id.to_owned(),
    }
}
