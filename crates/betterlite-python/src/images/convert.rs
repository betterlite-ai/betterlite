pub(crate) mod fields;
pub(crate) mod options;
mod tests;

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageFile;
use betterlite_openai::image_generation::reply::ImageGenerationReply;
use betterlite_sdk::call_files::{CallFile, CallFilesError, files_from_payload};
use betterlite_sdk::call_setup::RequestMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::time::{Duration, TryFromFloatSecsError};

pub(crate) const DEFAULT_IMAGE_NAME: &str = "image.png";
pub(crate) const DEFAULT_MASK_NAME: &str = "mask.png";
pub(crate) const DEFAULT_CONTENT_TYPE: &str = "application/octet-stream";
pub(crate) const DEFAULT_GENERATION_MODEL: &str = "dall-e-2";
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
pub(crate) const OPENAI_PROVIDER: &str = "openai";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ImageCallWire {
    pub(crate) arguments: RequestMap,
    pub(crate) settings: SettingsWire,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct SettingsWire {
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) openai_key: Option<SecretString>,
    pub(crate) drop_params: Option<bool>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConvertError {
    #[error("the native request text is not JSON of the expected shape: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the answer could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("{name} must be {expected}")]
    Field {
        name: String,
        expected: &'static str,
    },
    #[error("the timeout is not a number of seconds: {source}")]
    Timeout {
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error(transparent)]
    Files(#[from] CallFilesError),
    #[error("The following parameters are not supported for model {model}: {params}")]
    UnsupportedParams { model: String, params: String },
    #[error("{0}")]
    Unsupported(String),
}

#[derive(Debug, Serialize)]
pub(crate) struct ImageAnswer {
    #[serde(flatten)]
    pub(crate) reply: ImageGenerationReply,
    pub(crate) hidden_params: IndexMap<String, Json>,
}

impl ImageAnswer {
    pub(crate) fn new(reply: ImageGenerationReply, model: Option<&str>) -> Self {
        let mut hidden_params = IndexMap::new();
        if let Some(name) = model {
            hidden_params.insert(String::from("model"), Json::Text(name.to_owned()));
        }
        Self {
            reply,
            hidden_params,
        }
    }
}

pub(crate) fn parse(
    text: &str,
    blobs: &mut [Vec<u8>],
) -> Result<(ImageCallWire, Vec<(String, CallFile)>), ConvertError> {
    let wire: ImageCallWire =
        serde_json::from_str(text).map_err(|source| ConvertError::Json { source })?;
    let files = files_from_payload(&wire.arguments, blobs)?;
    Ok((wire, files))
}

pub(crate) fn images(files: &[(String, CallFile)], key: &str, fallback: &str) -> Vec<ImageFile> {
    let mut found = Vec::new();
    for (name, file) in files {
        if name == key {
            found.push(image_file(file, fallback));
        }
    }
    found
}

// an image_edit mask travels as a file, the way each image does
pub(crate) fn mask_file(files: &[(String, CallFile)]) -> Option<ImageFile> {
    images(files, "mask", DEFAULT_MASK_NAME).pop()
}

pub(crate) fn image_file(file: &CallFile, default_name: &str) -> ImageFile {
    let selected = if file.name.is_empty() || file.name == "file" {
        default_name.to_owned()
    } else {
        file.name.clone()
    };
    let content_type = file
        .content_type
        .clone()
        .unwrap_or_else(|| content_type_for(&selected));
    ImageFile {
        filename: Some(selected),
        content: file.bytes.clone(),
        content_type: Some(content_type),
    }
}

// the task's rule: the name's extension decides the type, never the bytes
pub(crate) fn content_type_for(name: &str) -> String {
    mime_guess::from_path(name)
        .first_raw()
        .map_or_else(|| DEFAULT_CONTENT_TYPE.to_owned(), str::to_owned)
}
