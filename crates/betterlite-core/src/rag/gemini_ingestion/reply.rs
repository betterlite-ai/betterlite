use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;
use tracing::warn;

const UPLOADED_PLACEHOLDER: &str = "uploaded";

#[derive(Debug, Error)]
pub enum ReplyError {
    #[error("the reply body is not valid JSON")]
    NotJson(#[source] serde_json::Error),
    #[error("the reply body is not a JSON object")]
    NotObject,
    #[error("the reply file is not a JSON object")]
    FileNotObject,
}

#[derive(Debug, Clone)]
pub struct UploadReply {
    pub file_id: String,
}

#[must_use]
pub fn decode_upload_reply(body: &[u8]) -> UploadReply {
    match upload_name(body) {
        Ok(file_id) => UploadReply { file_id },
        Err(cause) => {
            warn!("Could not parse upload response: {cause}");
            UploadReply {
                file_id: UPLOADED_PLACEHOLDER.to_owned(),
            }
        }
    }
}

pub(super) fn upload_name(body: &[u8]) -> Result<String, ReplyError> {
    let Json::Object(fields) = serde_json::from_slice::<Json>(body).map_err(ReplyError::NotJson)?
    else {
        return Err(ReplyError::NotObject);
    };
    if let Some(name) = top_name(&fields) {
        return Ok(name);
    }
    match fields.get("file") {
        None => Ok(String::new()),
        Some(Json::Object(file)) => Ok(nested_name(file)),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(ReplyError::FileNotObject)
        }
    }
}

fn top_name(fields: &IndexMap<String, Json>) -> Option<String> {
    match fields.get("name") {
        Some(Json::Text(name)) => (!name.is_empty()).then_some(name.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn nested_name(file: &IndexMap<String, Json>) -> String {
    match file.get("name") {
        Some(Json::Text(name)) => name.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => String::new(),
    }
}
