pub mod error;
pub mod reply;
pub mod request;

mod tests;

use http::HeaderMap;
use http::header::CONTENT_TYPE;
use secrecy::SecretString;
use serde_json::Value;

use self::error::WatsonxAudioError;
use self::request::{AUDIO_PATH, SUPPORTED_OPENAI_PARAMS, WatsonxTranscriptionRequest};
use crate::api::audio_file_input::ProcessedAudioFile;
use crate::openai_base_config::Object;
use crate::watsonx_auth::WatsonxAuth;
use crate::watsonx_auth::transport::WatsonxTransport;
use crate::watsonx_auth::types::WatsonxParams;
use crate::watsonx_auth::urls::{add_api_version_to_url, get_base_url};

pub struct WatsonxAudioConfig<'env, T> {
    pub auth: &'env WatsonxAuth<T>,
}

impl<T: WatsonxTransport> WatsonxAudioConfig<'_, T> {
    pub async fn validate_environment(
        &self,
        headers: HeaderMap,
        options: &Object,
        api_key: Option<&SecretString>,
    ) -> Result<HeaderMap, WatsonxAudioError> {
        let mut params: WatsonxParams = options.clone().into_iter().collect();
        let mut resolved = self
            .auth
            .validate_environment(headers, &mut params, api_key)
            .await?;
        let _content_type = resolved.remove(CONTENT_TYPE);
        Ok(resolved)
    }

    pub fn get_complete_url(
        &self,
        api_base: Option<&str>,
        options: &Object,
    ) -> Result<String, WatsonxAudioError> {
        let base = get_base_url(api_base, &self.auth.environment)?;
        let url = format!("{}{AUDIO_PATH}", base.trim_end_matches('/'));
        let version = match options.get("api_version") {
            None | Some(Value::Null) => None,
            Some(Value::String(version)) => Some(version.as_str()),
            Some(_) => {
                return Err(WatsonxAudioError::NotText {
                    field: "api_version",
                });
            }
        };
        Ok(add_api_version_to_url(&url, version))
    }

    // model first, then the project or space the caller named, then each supported option that is set
    pub fn transform_audio_request(
        &self,
        model: &str,
        file: ProcessedAudioFile,
        options: &Object,
    ) -> Result<WatsonxTranscriptionRequest, WatsonxAudioError> {
        let mut fields = vec![(String::from("model"), Value::String(model.to_owned()))];
        let named_project = match text_option(options, "project_id")? {
            Some(id) => Some(id),
            None => text_option(options, "watsonx_project")?,
        };
        match named_project {
            Some(project_id) => {
                fields.push((String::from("project_id"), Value::String(project_id)));
            }
            None => {
                if let Some(space_id) = text_option(options, "space_id")? {
                    fields.push((String::from("space_id"), Value::String(space_id)));
                }
            }
        }
        for (name, value) in options {
            if SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) && !value.is_null() {
                fields.push((name.clone(), value.clone()));
            }
        }
        Ok(WatsonxTranscriptionRequest { fields, file })
    }
}

fn text_option(options: &Object, field: &'static str) -> Result<Option<String>, WatsonxAudioError> {
    match options.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(WatsonxAudioError::NotText { field }),
    }
}
