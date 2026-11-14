use betterlite_prompts::arguments::json::Json;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub outputs: String,
}

#[derive(Debug, Error)]
pub enum PetalsReplyError {
    #[error("the petals reply is not JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the petals reply is not a JSON object")]
    NotAnObject,
    #[error("the petals reply has no outputs field")]
    MissingOutputs,
    #[error("the petals reply outputs is not a string")]
    NotAString,
    #[error("the petals reply outputs is empty")]
    EmptyOutputs,
}

// handler.py keeps response.json()["outputs"] only when it is a non-empty string and crashes on every other shape (completion/handler.py L81-L131)
pub fn decode_petals_reply(bytes: &[u8]) -> Result<Reply, PetalsReplyError> {
    let shaped: Json =
        serde_json::from_slice(bytes).map_err(|source| PetalsReplyError::Json { source })?;
    let Json::Object(fields) = shaped else {
        return Err(PetalsReplyError::NotAnObject);
    };
    let Some(outputs) = fields.get("outputs") else {
        return Err(PetalsReplyError::MissingOutputs);
    };
    let Json::Text(text) = outputs else {
        return Err(PetalsReplyError::NotAString);
    };
    if text.is_empty() {
        return Err(PetalsReplyError::EmptyOutputs);
    }
    Ok(Reply {
        outputs: text.clone(),
    })
}
