use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageHeaders, ImageHttpResponse,
};
use indexmap::IndexMap;
use thiserror::Error;

// LiteLLM raises its error class for a body it cannot parse and a plain Python exception for every other reply shape
#[derive(Debug, Error)]
pub enum RecraftReplyError {
    #[error("the Recraft reply is not JSON")]
    Malformed {
        status: u16,
        headers: ImageHeaders,
        #[source]
        source: serde_json::Error,
    },
    #[error("the Recraft reply is not a JSON object")]
    NotObject,
    #[error("the Recraft reply has no data field")]
    MissingData,
    #[error("the Recraft reply data is not a list")]
    DataShape,
    #[error("the Recraft reply image is not an object")]
    ImageShape,
    #[error("the Recraft reply field {field} is not text or null")]
    ImageField { field: &'static str },
}

impl RecraftReplyError {
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Malformed { status, .. } => Some(*status),
            Self::NotObject
            | Self::MissingData
            | Self::DataShape
            | Self::ImageShape
            | Self::ImageField { .. } => None,
        }
    }

    #[must_use]
    pub const fn headers(&self) -> Option<&ImageHeaders> {
        match self {
            Self::Malformed { headers, .. } => Some(headers),
            Self::NotObject
            | Self::MissingData
            | Self::DataShape
            | Self::ImageShape
            | Self::ImageField { .. } => None,
        }
    }
}

pub fn decode_recraft_images(
    response: &ImageHttpResponse,
) -> Result<Vec<ImageData>, RecraftReplyError> {
    let value: Json =
        serde_json::from_slice(&response.body).map_err(|source| RecraftReplyError::Malformed {
            status: response.status_code,
            headers: response.headers.clone(),
            source,
        })?;
    let Json::Object(mut fields) = value else {
        return Err(RecraftReplyError::NotObject);
    };
    let Some(data) = fields.shift_remove("data") else {
        return Err(RecraftReplyError::MissingData);
    };
    let Json::List(entries) = data else {
        return Err(RecraftReplyError::DataShape);
    };
    entries.iter().map(image_data).collect()
}

fn image_data(value: &Json) -> Result<ImageData, RecraftReplyError> {
    let Json::Object(fields) = value else {
        return Err(RecraftReplyError::ImageShape);
    };
    Ok(ImageData {
        b64_json: text_field(fields, "b64_json")?,
        url: text_field(fields, "url")?,
        revised_prompt: None,
    })
}

fn text_field(
    fields: &IndexMap<String, Json>,
    name: &'static str,
) -> Result<Option<String>, RecraftReplyError> {
    match fields.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(RecraftReplyError::ImageField { field: name }),
    }
}
