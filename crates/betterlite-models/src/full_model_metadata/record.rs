use std::sync::{Arc, Mutex};

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::value::RawValue;

use crate::decisions::Json;
use crate::provider_metadata_factory::adapters::ProviderSpecificModelInfo;

use super::error::ModelRecordError;

// A cache hit returns this same handle, so a caller's mutation shows up on the next hit like the source dict.
pub type ModelInfoHandle = Arc<Mutex<ModelInfo>>;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ModelInfo {
    pub fields: IndexMap<String, Json>,
}

impl ModelInfo {
    pub fn parse(metadata: &RawValue) -> Result<Self, ModelRecordError> {
        let parsed = serde_json::from_str::<Json>(metadata.get())
            .map_err(|source| ModelRecordError::Json { source })?;
        match parsed {
            Json::Object(fields) => Ok(Self { fields }),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                Err(ModelRecordError::NotAnObject)
            }
        }
    }
}

pub(super) fn build(
    metadata: &RawValue,
    overlay: Option<&ProviderSpecificModelInfo>,
    supported_openai_params: Option<Vec<String>>,
) -> Result<ModelInfo, ModelRecordError> {
    let mut info = ModelInfo::parse(metadata)?;
    if let Some(configured) = overlay {
        let entries = [
            (
                "supports_function_calling",
                configured.supports_function_calling,
            ),
            (
                "supports_prompt_caching",
                configured.supports_prompt_caching,
            ),
            ("supports_reasoning", configured.supports_reasoning),
            ("supports_vision", configured.supports_vision),
            ("supports_pdf_input", configured.supports_pdf_input),
            ("supports_web_search", configured.supports_web_search),
        ];
        for (name, setting) in entries {
            if let Some(present) = setting {
                info.fields.insert(name.to_owned(), Json::Bool(present));
            }
        }
    }
    // the source builds ModelInfo(**_model_info, supported_openai_params=...), which rejects a duplicate key
    if info.fields.contains_key("supported_openai_params") {
        return Err(ModelRecordError::DuplicateSupportedParameters);
    }
    let params = supported_openai_params.map_or_else(
        || Json::Null,
        |names| Json::List(names.into_iter().map(Json::Text).collect()),
    );
    info.fields
        .insert("supported_openai_params".to_owned(), params);
    Ok(info)
}
