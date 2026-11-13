use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageHeaders, ImageHttpResponse, ImageResponse,
};
use serde::Deserialize;
use thiserror::Error;

const CONTENT_FILTERED: &str = "CONTENT_FILTERED";
const FILTERED_STATUS: u16 = 400;

#[derive(Debug, Error)]
pub enum StabilityReplyError {
    #[error("the Stability AI reply does not fit a generation response")]
    Malformed {
        status: u16,
        headers: ImageHeaders,
        #[source]
        source: serde_json::Error,
    },
    #[error("the Stability AI reply is not a JSON object")]
    NotObject { status: u16, headers: ImageHeaders },
    #[error("Stability AI reported an error: {errors:?}")]
    Upstream {
        status: u16,
        headers: ImageHeaders,
        errors: Json,
    },
    #[error("Stability AI filtered the content")]
    ContentFiltered { headers: ImageHeaders },
}

impl StabilityReplyError {
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::Malformed { status, .. }
            | Self::NotObject { status, .. }
            | Self::Upstream { status, .. } => *status,
            Self::ContentFiltered { .. } => FILTERED_STATUS,
        }
    }

    #[must_use]
    pub const fn headers(&self) -> &ImageHeaders {
        match self {
            Self::Malformed { headers, .. }
            | Self::NotObject { headers, .. }
            | Self::Upstream { headers, .. }
            | Self::ContentFiltered { headers } => headers,
        }
    }
}

#[derive(Deserialize)]
struct ReplyFields {
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    finish_reason: Option<String>,
}

pub fn decode_stability_image(
    response: &ImageHttpResponse,
    model_response: &mut ImageResponse,
) -> Result<(), StabilityReplyError> {
    let value: Json = serde_json::from_slice(&response.body).map_err(|source| {
        StabilityReplyError::Malformed {
            status: response.status_code,
            headers: response.headers.clone(),
            source,
        }
    })?;
    let Json::Object(fields) = value else {
        return Err(StabilityReplyError::NotObject {
            status: response.status_code,
            headers: response.headers.clone(),
        });
    };
    if let Some(errors) = fields.get("errors") {
        return Err(StabilityReplyError::Upstream {
            status: response.status_code,
            headers: response.headers.clone(),
            errors: errors.clone(),
        });
    }
    let reply: ReplyFields = serde_json::from_slice(&response.body).map_err(|source| {
        StabilityReplyError::Malformed {
            status: response.status_code,
            headers: response.headers.clone(),
            source,
        }
    })?;
    if reply.finish_reason.as_deref() == Some(CONTENT_FILTERED) {
        return Err(StabilityReplyError::ContentFiltered {
            headers: response.headers.clone(),
        });
    }
    let data = model_response.data.get_or_insert_with(Vec::new);
    if let Some(image) = reply.image.filter(|image| !image.is_empty()) {
        data.push(ImageData {
            b64_json: Some(image),
            url: None,
            revised_prompt: None,
        });
    }
    Ok(())
}
